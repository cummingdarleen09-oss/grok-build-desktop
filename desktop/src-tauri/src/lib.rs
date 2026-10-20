//! Tauri shell around [`grok_desktop_core`].
//!
//! The window is the desktop workbench (sessions, projects, media, automations).
//! Model traffic goes through [`grok_providers`]. Official Grok sign-in uses
//! [`xai_grok_login`] and stores the session in the same auth file as the CLI.
//! The CLI still builds as its own binary.
//!
//! Every command is `async`. Tauri runs a plain `fn` command on the main
//! thread, where file scans, keychain reads, `git`, and folder pickers freeze
//! the window. Blocking work goes through [`blocking`].

mod account;
mod diag;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use grok_desktop_core::{
    AgentEvent, Automation, DesktopSettings, ExtensionRecord, McpPool, MediaFile, ModelDefault,
    Session, SessionSummary, TurnControl, TurnOptions, branch_session, create_session,
    delete_session, discover_extensions, list_media, list_sessions, load_automations,
    load_classic_session, load_session, load_settings, model_default_key, process, rename_session,
    render_mermaid, run_turn, save_automations, save_session, save_settings, tr,
};
use grok_providers::{
    ModelListAuth, ProviderClient, ProviderConfig, ProviderFormat, ProviderStore, ReasoningEffort,
    ReasoningEffortOption, SecretLocation, SecretStore, Turn, XaiSecretSource, apply_choice,
    apply_known_models, fetch_provider_models, has_login_session, load_or_default_store,
    normalize_store, refresh_xai_model_list, save_provider_store, session_identity,
    sync_cli_config,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Blocking work slower than this is written to the diagnostics log.
const SLOW: Duration = Duration::from_millis(250);
const XAI_API_BASE_URL: &str = "https://api.x.ai/v1";

pub struct AppState {
    client: ProviderClient,
    secrets: SecretStore,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    approvals: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    turn: tokio::sync::Mutex<()>,
    mcp: tokio::sync::Mutex<Option<McpPool>>,
    models_refresh_started: AtomicBool,
}

#[derive(Serialize)]
struct SecretStatus {
    id: String,
    location: Option<&'static str>,
    from_env: bool,
    from_session: bool,
}

#[derive(Serialize)]
struct Bootstrap {
    settings: DesktopSettings,
    providers: ProviderStore,
    sessions: Vec<SessionSummary>,
    extensions: Vec<ExtensionRecord>,
    secrets: Vec<SecretStatus>,
    grok_home: String,
    user_name: String,
    automations: Vec<Automation>,
    log_path: String,
}

#[derive(Serialize, Clone)]
struct AgentEnvelope {
    session_id: String,
    event: AgentEvent,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run `job` on the blocking pool and log it when it is slow or fails.
async fn blocking<T, F>(label: &'static str, job: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|error| format!("{label} stopped: {error}"))?;
    let elapsed = started.elapsed();
    if elapsed > SLOW {
        diag::warn(&format!("{label} took {} ms", elapsed.as_millis()));
    }
    if let Err(error) = &result
        && error != "cancelled"
    {
        diag::warn(&format!("{label} failed: {error}"));
    }
    result
}

fn secret_status(secrets: &SecretStore, provider: &ProviderConfig) -> SecretStatus {
    let location = secrets
        .location(&provider.id)
        .ok()
        .flatten()
        .map(|location| match location {
            SecretLocation::Keyring => "keyring",
            SecretLocation::File => "file",
        });
    let from_env = provider.env_names().into_iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    });
    SecretStatus {
        id: provider.id.clone(),
        location,
        from_env,
        from_session: provider.id == "xai" && has_login_session(),
    }
}

fn secret_statuses(secrets: &SecretStore, store: &ProviderStore) -> Vec<SecretStatus> {
    store
        .providers
        .iter()
        .map(|provider| secret_status(secrets, provider))
        .collect()
}

/// Startup data. It never waits on the network: the xAI model list comes
/// from the cache here and is refreshed in the background once per launch.
#[tauri::command]
async fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let started = Instant::now();
    let secrets = state.secrets.clone();
    let boot = blocking("bootstrap", move || {
        let mut providers = load_or_default_store();
        apply_known_models(&mut providers);
        let sessions = list_sessions().unwrap_or_else(|error| {
            diag::error(&format!("listing chats failed: {error}"));
            Vec::new()
        });
        Ok(Bootstrap {
            settings: load_settings(),
            secrets: secret_statuses(&secrets, &providers),
            providers,
            sessions,
            extensions: discover_extensions(),
            grok_home: grok_providers::grok_home().display().to_string(),
            user_name: user_name(),
            automations: load_automations(),
            log_path: diag::log_path().display().to_string(),
        })
    })
    .await?;
    diag::info(&format!(
        "bootstrap {} ms: {} chats, {} providers, active {:?}",
        started.elapsed().as_millis(),
        boot.sessions.len(),
        boot.providers.providers.len(),
        boot.settings.active_provider
    ));
    if !state.models_refresh_started.swap(true, Ordering::SeqCst) {
        tauri::async_runtime::spawn(refresh_models(app));
    }
    Ok(boot)
}

/// Refresh the xAI model list without holding up the window, then tell the
/// window about the result.
async fn refresh_models(app: AppHandle) {
    let secrets = app.state::<AppState>().secrets.clone();
    let prepared = blocking("models.prepare", move || {
        let store = load_or_default_store();
        let xai = store
            .providers
            .iter()
            .find(|provider| provider.id == "xai")
            .cloned();
        let found = match &xai {
            Some(xai) => secrets.resolve_with_source(xai, None).ok().flatten(),
            None => None,
        };
        Ok((xai, found, session_identity()))
    })
    .await;
    let Ok((Some(xai), Some((key, source)), identity)) = prepared else {
        return;
    };
    let mut routed = xai.clone();
    route_xai_session(&mut routed, source);
    let auth = if routed.base_url != xai.base_url {
        ModelListAuth::Session {
            token: &key,
            user_id: identity.user_id.as_deref(),
            email: identity.email.as_deref(),
            client_version: env!("CARGO_PKG_VERSION"),
        }
    } else {
        ModelListAuth::ApiKey {
            base_url: &xai.base_url,
            key: &key,
        }
    };
    let started = Instant::now();
    let choice =
        match tokio::time::timeout(Duration::from_secs(8), refresh_xai_model_list(auth)).await {
            Ok(choice) => choice,
            Err(_) => {
                diag::warn("xAI model refresh timed out; using the cached list");
                return;
            }
        };
    diag::info(&format!(
        "xAI model refresh ({source:?}) {} ms, newest {}, {} models",
        started.elapsed().as_millis(),
        choice.latest,
        choice.models.len()
    ));
    let secrets = app.state::<AppState>().secrets.clone();
    let saved = blocking("models.save", move || {
        let mut store = load_or_default_store();
        let before = store.clone();
        apply_choice(&mut store, &choice);
        if store != before {
            save_provider_store(&store)?;
            sync_cli_config(&store.providers)?;
        }
        let statuses = secret_statuses(&secrets, &store);
        Ok((store, statuses))
    })
    .await;
    if let Ok((store, statuses)) = saved {
        let _ = app.emit(
            "providers-updated",
            serde_json::json!({ "providers": store, "secrets": statuses }),
        );
    }
}

#[tauri::command]
async fn list_chats() -> Result<Vec<SessionSummary>, String> {
    blocking("list_chats", list_sessions).await
}

fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Grok".into())
}

#[tauri::command]
async fn save_desktop_settings(mut settings: DesktopSettings) -> Result<(), String> {
    blocking("save_settings", move || {
        // Only set_chat_options writes these; the window's copy may be stale.
        settings.model_defaults = load_settings().model_defaults;
        save_settings(&settings)
    })
    .await
}

#[tauri::command]
async fn save_providers(store: ProviderStore) -> Result<ProviderStore, String> {
    blocking("save_providers", move || {
        for provider in &store.providers {
            provider.validate()?;
        }
        let store = normalize_store(store);
        save_provider_store(&store)?;
        if let Err(error) = sync_cli_config(&store.providers) {
            diag::warn(&format!("CLI config was not updated: {error}"));
        }
        Ok(store)
    })
    .await
}

#[tauri::command]
async fn set_secret(
    state: State<'_, AppState>,
    id: String,
    secret: String,
) -> Result<&'static str, String> {
    let secrets = state.secrets.clone();
    let location = blocking("set_secret", move || secrets.set(&id, &secret)).await?;
    Ok(match location {
        SecretLocation::Keyring => "keyring",
        SecretLocation::File => "file",
    })
}

#[tauri::command]
async fn clear_secret(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let secrets = state.secrets.clone();
    blocking("clear_secret", move || secrets.delete(&id)).await
}

#[tauri::command]
async fn create_chat(
    cwd: String,
    provider_id: String,
    model: String,
    lang: String,
) -> Result<Session, String> {
    blocking("create_chat", move || {
        let session = create_session(&cwd, &provider_id, &model, &tr(&lang, "new_chat"));
        save_session(&session)?;
        Ok(session)
    })
    .await
}

#[tauri::command]
async fn open_chat(id: String) -> Result<Session, String> {
    blocking("open_chat", move || load_session(&id)).await
}

#[tauri::command]
async fn remove_chat(id: String) -> Result<(), String> {
    blocking("remove_chat", move || delete_session(&id)).await
}

#[tauri::command]
async fn rename_chat(id: String, title: String) -> Result<Session, String> {
    blocking("rename_chat", move || rename_session(&id, &title)).await
}

#[tauri::command]
async fn branch_chat(
    session_id: String,
    message_id: String,
    include_message: bool,
    lang: String,
) -> Result<Session, String> {
    blocking("branch_chat", move || {
        let parent = load_session(&session_id)?;
        let title = format!("{} · {}", tr(&lang, "branch_title"), parent.title);
        branch_session(&session_id, &message_id, include_message, &title)
    })
    .await
}

#[tauri::command]
async fn pick_project_folder() -> Result<String, String> {
    blocking("pick_project_folder", || match pick_folder() {
        Some(path) if !path.trim().is_empty() => Ok(path),
        Some(_) | None => Err("cancelled".into()),
    })
    .await
}

fn pick_folder() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return picker_stdout(
            "osascript",
            &[
                "-e",
                r#"POSIX path of (choose folder with prompt "Open project")"#,
            ],
        )
        .ok()
        .flatten();
    }
    #[cfg(target_os = "windows")]
    {
        // UTF-8 output keeps non-ASCII folder names intact; the console code
        // page (GBK on Chinese Windows) would garble them. The TopMost owner
        // keeps the dialog in front of the app window.
        return picker_stdout(
            "powershell",
            &[
                "-NoProfile",
                "-STA",
                "-Command",
                "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $owner = New-Object System.Windows.Forms.Form -Property @{TopMost = $true}; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Open project'; if ($d.ShowDialog($owner) -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $d.SelectedPath }",
            ],
        )
        .ok()
        .flatten();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        match picker_stdout(
            "zenity",
            &["--file-selection", "--directory", "--title=Open project"],
        ) {
            Ok(path) => path,
            Err(()) => picker_stdout("kdialog", &["--getexistingdirectory", "."])
                .ok()
                .flatten(),
        }
    }
}

/// `Ok(None)` is a cancel. `Err(())` means the picker program is not installed.
fn picker_stdout(program: &str, args: &[&str]) -> Result<Option<String>, ()> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    process::hide_window(&mut command);
    let output = command.output().map_err(|_| ())?;
    if !output.status.success() {
        return Ok(None);
    }
    let mut text = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_start_matches('\u{feff}')
        .to_string();
    while text.len() > 1 && (text.ends_with('/') || text.ends_with('\\')) {
        text.pop();
    }
    Ok(if text.is_empty() { None } else { Some(text) })
}

/// The provider a chat should call, or a message saying why there is none.
fn pick_provider(
    store: &ProviderStore,
    settings: &DesktopSettings,
    provider_id: &str,
) -> Result<ProviderConfig, String> {
    let lang = &settings.lang;
    let wanted = if provider_id.trim().is_empty() {
        settings.active_provider.clone().unwrap_or_default()
    } else {
        provider_id.to_string()
    };
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == wanted)
        .or_else(|| {
            store
                .providers
                .iter()
                .find(|provider| provider.enabled && wanted.is_empty())
        })
        .cloned()
        .ok_or_else(|| tr(lang, "no_provider"))?;
    if !provider.enabled {
        return Err(tr(lang, "provider_disabled").replace("{name}", &provider.name));
    }
    Ok(provider)
}

fn missing_key(lang: &str, provider: &ProviderConfig) -> String {
    tr(lang, "missing_key_for").replace("{name}", &provider.name)
}

/// An Official Grok OAuth session is accepted by the CLI chat proxy, not by
/// api.x.ai; only API keys go to api.x.ai. Same routing and headers as the CLI.
fn route_xai_session(provider: &mut ProviderConfig, source: XaiSecretSource) {
    if provider.id != "xai" || source != XaiSecretSource::Session {
        return;
    }
    let base = provider.base_url.trim().trim_end_matches('/');
    if !base.eq_ignore_ascii_case(XAI_API_BASE_URL) {
        return;
    }
    provider.base_url = xai_grok_login::CLI_CHAT_PROXY_BASE_URL_DEFAULT.to_string();
    for (name, value) in [
        ("X-XAI-Token-Auth", "xai-grok-cli"),
        ("x-authenticateresponse", "authenticate-response"),
        ("x-grok-client-mode", "interactive"),
    ] {
        provider
            .extra_headers
            .entry(name.to_string())
            .or_insert_with(|| value.to_string());
    }
}

/// Resolve a key for `provider`. An expired Grok session with a refresh token
/// is refreshed first.
async fn credential_for(
    secrets: &SecretStore,
    provider: &ProviderConfig,
    lang: &str,
) -> Result<(String, XaiSecretSource), String> {
    let lookup = {
        let secrets = secrets.clone();
        let provider = provider.clone();
        move || secrets.resolve_with_source(&provider, None)
    };
    if let Some(found) = blocking("resolve_key", lookup.clone()).await? {
        return Ok(found);
    }
    if provider.id == "xai" {
        let profile = blocking("read_login", || Ok(grok_providers::read_login_profile())).await?;
        if profile.signed_in && profile.expired && profile.has_refresh {
            diag::info("Grok session expired; refreshing before the request");
            if account::refresh_session().await
                && let Some(found) = blocking("resolve_key", lookup).await?
            {
                return Ok(found);
            }
        }
    }
    Err(missing_key(lang, provider))
}

/// Check that a chat with `provider_id` could be sent, without sending it.
#[tauri::command]
async fn check_ready(state: State<'_, AppState>, provider_id: String) -> Result<(), String> {
    let (settings, provider) = blocking("check_ready", move || {
        let settings = load_settings();
        let store = load_or_default_store();
        let provider = pick_provider(&store, &settings, &provider_id)?;
        Ok((settings, provider))
    })
    .await?;
    if provider.id == "xai" {
        let profile = blocking("read_login", || Ok(grok_providers::read_login_profile())).await?;
        if profile.signed_in && profile.expired && profile.has_refresh {
            return Ok(());
        }
    }
    let secrets = state.secrets.clone();
    let lookup = {
        let provider = provider.clone();
        move || secrets.resolve_with_source(&provider, None)
    };
    match blocking("resolve_key", lookup).await? {
        Some(_) => Ok(()),
        None => Err(missing_key(&settings.lang, &provider)),
    }
}

struct PreparedTurn {
    session: Session,
    settings: DesktopSettings,
    provider: ProviderConfig,
}

#[tauri::command]
async fn send_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    text: String,
    plan: Option<bool>,
    model: Option<String>,
) -> Result<Session, String> {
    let started = Instant::now();
    let plan = plan.unwrap_or(false);
    let id = session_id.clone();
    let PreparedTurn {
        mut session,
        settings,
        mut provider,
    } = blocking("prepare_turn", move || {
        let mut session = load_session(&id)?;
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            session.model = model;
        }
        let settings = load_settings();
        let store = load_or_default_store();
        let mut provider = pick_provider(&store, &settings, &session.provider_id)?;
        if !session.model.trim().is_empty() {
            provider.model = session.model.clone();
        }
        if session.cwd.trim().is_empty() {
            session.cwd = settings.cwd.clone();
        }
        if !Path::new(&session.cwd).is_dir() {
            return Err(tr(&settings.lang, "cwd_missing").replace("{path}", &session.cwd));
        }
        Ok(PreparedTurn {
            session,
            settings,
            provider,
        })
    })
    .await?;
    let lang = settings.lang.clone();
    let Ok(_turn) = state.turn.try_lock() else {
        return Err(tr(&lang, "busy"));
    };
    let options = TurnOptions::resolve(
        &provider,
        &session,
        &settings.model_default(&provider.id, &provider.model),
    );
    let (api_key, source) = credential_for(&state.secrets, &provider, &lang).await?;
    route_xai_session(&mut provider, source);
    diag::info(&format!(
        "send: chat {session_id} provider {} ({:?}) model {} via {} key {:?}, plan {plan}, effort {:?}, window {} (compact at {}%)",
        provider.id,
        provider.format,
        provider.model,
        provider.base_url,
        source,
        options.effort,
        options.context_window,
        options.compact_at_percent
    ));
    let emit = |event: AgentEvent| {
        let _ = app.emit(
            "agent-event",
            AgentEnvelope {
                session_id: session_id.clone(),
                event,
            },
        );
    };

    let mut idle_pool = McpPool::empty();
    let mut mcp_slot = state.mcp.lock().await;
    let pool: &mut McpPool = if plan {
        &mut idle_pool
    } else {
        let path = McpPool::config_path();
        let current = match mcp_slot.as_mut() {
            Some(pool) => pool.is_current(&path),
            None => false,
        };
        if !current {
            *mcp_slot = None;
            let configured = McpPool::configured_servers(&path);
            if !configured.is_empty() {
                emit(AgentEvent::Status {
                    text: tr(&lang, "mcp_connecting"),
                });
            }
            let connect_started = Instant::now();
            let fresh = McpPool::connect_file(&path).await;
            if !configured.is_empty() {
                diag::info(&format!(
                    "MCP: {} of {} servers started in {} ms",
                    configured.len() - fresh.failures.len(),
                    configured.len(),
                    connect_started.elapsed().as_millis()
                ));
            }
            for (name, error) in &fresh.failures {
                diag::warn(&format!("MCP server {name} did not start: {error}"));
                emit(AgentEvent::Status {
                    text: tr(&lang, "mcp_failed")
                        .replace("{name}", name)
                        .replace("{error}", error),
                });
            }
            *mcp_slot = Some(fresh);
        }
        mcp_slot.get_or_insert_with(McpPool::empty)
    };

    emit(AgentEvent::Status {
        text: tr(&lang, "waiting_model").replace("{name}", &provider.name),
    });
    let cancel = Arc::new(AtomicBool::new(false));
    *lock(&state.cancel) = Some(cancel.clone());
    let approvals = &state.approvals;
    let request_started = Instant::now();
    let mut first_output: Option<Duration> = None;
    let result = run_turn(
        &mut session,
        &provider,
        &api_key,
        &text,
        &lang,
        settings.asks_permission(),
        plan,
        &settings.disabled_tools,
        &state.client,
        &TurnControl { cancel },
        options,
        pool,
        |event| {
            if first_output.is_none()
                && matches!(
                    event,
                    AgentEvent::TextDelta { .. }
                        | AgentEvent::ReasoningDelta { .. }
                        | AgentEvent::ToolStarted { .. }
                        | AgentEvent::ApprovalNeeded { .. }
                )
            {
                first_output = Some(request_started.elapsed());
            }
            if let AgentEvent::Error { message } = &event {
                diag::warn(&format!("turn event error: {message}"));
            }
            emit(event);
        },
        |id, _name, _arguments| {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            lock(approvals).insert(id, sender);
            async move { receiver.await.unwrap_or(false) }
        },
    )
    .await;
    drop(mcp_slot);
    *lock(&state.cancel) = None;
    for (_, sender) in lock(&state.approvals).drain() {
        let _ = sender.send(false);
    }
    let saved = blocking("save_turn", move || {
        save_session(&session)?;
        Ok(session)
    })
    .await;
    let outcome = match &result {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("error: {error}"),
    };
    diag::info(&format!(
        "turn {outcome}; first output {} ms, total {} ms",
        first_output.map_or("-".into(), |elapsed| elapsed.as_millis().to_string()),
        started.elapsed().as_millis()
    ));
    if let Err(error) = result {
        return Err(tr(&lang, "request_failed")
            .replace("{name}", &provider.name)
            .replace("{error}", &error));
    }
    saved
}

#[tauri::command]
async fn cancel_prompt(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(flag) = lock(&state.cancel).as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    for (_, sender) in lock(&state.approvals).drain() {
        let _ = sender.send(false);
    }
    Ok(())
}

#[tauri::command]
async fn resolve_approval(
    state: State<'_, AppState>,
    id: String,
    approved: bool,
) -> Result<(), String> {
    let sender = lock(&state.approvals)
        .remove(&id)
        .ok_or_else(|| "no pending approval".to_string())?;
    sender
        .send(approved)
        .map_err(|_| "approval already finished".to_string())
}

#[tauri::command]
async fn test_provider(
    state: State<'_, AppState>,
    id: String,
    model: Option<String>,
) -> Result<String, String> {
    let (settings, mut provider) = blocking("test_provider", move || {
        let settings = load_settings();
        let store = load_or_default_store();
        let mut provider = store
            .providers
            .into_iter()
            .find(|provider| provider.id == id)
            .ok_or_else(|| tr(&settings.lang, "no_provider"))?;
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            provider.model = model;
        }
        Ok((settings, provider))
    })
    .await?;
    let (api_key, source) = credential_for(&state.secrets, &provider, &settings.lang).await?;
    route_xai_session(&mut provider, source);
    provider.max_tokens = 32;
    let started = Instant::now();
    let result = state
        .client
        .complete_text(
            &provider,
            &api_key,
            &[Turn::user("Reply with the single word pong.")],
        )
        .await
        .map_err(|error| error.detailed());
    diag::info(&format!(
        "test {} via {} in {} ms: {}",
        provider.id,
        provider.base_url,
        started.elapsed().as_millis(),
        match &result {
            Ok(_) => "ok".to_string(),
            Err(error) => error.clone(),
        }
    ));
    result
}

#[tauri::command]
async fn set_chat_model(id: String, provider_id: String, model: String) -> Result<Session, String> {
    blocking("set_chat_model", move || {
        let mut session = load_session(&id)?;
        if session.provider_id != provider_id || session.model != model {
            session.reasoning_effort = None;
            session.context_window = None;
        }
        session.provider_id = provider_id;
        session.model = model;
        save_session(&session)?;
        Ok(session)
    })
    .await
}

/// What the composer offers for one model and what the next request uses.
#[derive(Serialize)]
struct ChatOptions {
    efforts: Vec<ReasoningEffortOption>,
    effort: Option<ReasoningEffort>,
    context_windows: Vec<u64>,
    context_window: u64,
    compact_at_percent: u8,
}

/// The chat's own choice applies only while the chat still uses `model`.
fn chat_choice(session: Option<&Session>, provider_id: &str, model: &str) -> ModelDefault {
    match session {
        Some(session) if session.provider_id == provider_id && session.model == model => {
            ModelDefault {
                reasoning_effort: session.reasoning_effort,
                context_window: session.context_window,
            }
        }
        _ => ModelDefault::default(),
    }
}

fn resolve_chat_options(
    settings: &DesktopSettings,
    session: Option<&Session>,
    provider_id: &str,
    model: &str,
) -> Result<ChatOptions, String> {
    let store = load_or_default_store();
    let mut provider = store
        .providers
        .into_iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| tr(&settings.lang, "no_provider"))?;
    if !model.trim().is_empty() {
        provider.model = model.to_string();
    }
    let menu = provider.model_options(&provider.model);
    let chosen = TurnOptions::choose(
        &provider,
        &chat_choice(session, provider_id, &provider.model),
        &settings.model_default(provider_id, &provider.model),
    );
    Ok(ChatOptions {
        efforts: menu.efforts,
        effort: chosen.effort,
        context_windows: menu.context_windows,
        context_window: chosen.context_window,
        compact_at_percent: chosen.compact_at_percent,
    })
}

#[tauri::command]
async fn chat_options(
    session_id: Option<String>,
    provider_id: String,
    model: String,
) -> Result<ChatOptions, String> {
    blocking("chat_options", move || {
        let settings = load_settings();
        let session = session_id
            .filter(|id| !id.is_empty())
            .and_then(|id| load_session(&id).ok());
        resolve_chat_options(&settings, session.as_ref(), &provider_id, &model)
    })
    .await
}

/// Save the effort and/or context window for the chat (when there is one)
/// and as the default for this model in new chats.
#[tauri::command]
async fn set_chat_options(
    session_id: Option<String>,
    provider_id: String,
    model: String,
    effort: Option<ReasoningEffort>,
    context_window: Option<u64>,
) -> Result<ChatOptions, String> {
    blocking("set_chat_options", move || {
        let mut settings = load_settings();
        let saved = settings
            .model_defaults
            .entry(model_default_key(&provider_id, &model))
            .or_default();
        if effort.is_some() {
            saved.reasoning_effort = effort;
        }
        if context_window.is_some() {
            saved.context_window = context_window;
        }
        save_settings(&settings)?;
        let mut session = session_id
            .filter(|id| !id.is_empty())
            .and_then(|id| load_session(&id).ok());
        if let Some(session) = session.as_mut()
            && session.provider_id == provider_id
            && session.model == model
        {
            if effort.is_some() {
                session.reasoning_effort = effort;
            }
            if context_window.is_some() {
                session.context_window = context_window;
            }
            save_session(session)?;
        }
        let options = resolve_chat_options(&settings, session.as_ref(), &provider_id, &model)?;
        diag::info(&format!(
            "chat options {provider_id}/{model}: effort {:?}, window {}",
            options.effort, options.context_window
        ));
        Ok(options)
    })
    .await
}

#[tauri::command]
async fn fetch_models(
    state: State<'_, AppState>,
    id: String,
    base_url: String,
    format: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    let format = match format.as_str() {
        "anthropic" => ProviderFormat::Anthropic,
        "responses" => ProviderFormat::Responses,
        _ => ProviderFormat::OpenAi,
    };
    let auth = if format == ProviderFormat::Anthropic {
        grok_providers::AuthStyle::XApiKey
    } else {
        grok_providers::AuthStyle::Bearer
    };
    let key = if api_key.trim().is_empty() {
        let secrets = state.secrets.clone();
        blocking("fetch_models.key", move || {
            let store = load_or_default_store();
            match store
                .providers
                .into_iter()
                .find(|provider| provider.id == id)
            {
                Some(provider) => Ok(secrets.resolve(&provider, None)?.unwrap_or_default()),
                None => Ok(String::new()),
            }
        })
        .await?
    } else {
        api_key
    };
    if key.trim().is_empty() {
        return Err("missing api key".into());
    }
    fetch_provider_models(&base_url, auth, &key).await
}

#[tauri::command]
async fn open_classic(id: String) -> Result<Session, String> {
    blocking("open_classic", move || load_classic_session(&id)).await
}

#[tauri::command]
async fn render_diagram(source: String, dark: bool) -> Result<String, String> {
    blocking("render_diagram", move || render_mermaid(&source, dark)).await
}

#[tauri::command]
async fn project_media(cwd: String) -> Result<Vec<MediaFile>, String> {
    blocking("project_media", move || Ok(list_media(&cwd))).await
}

#[tauri::command]
async fn load_automation_list() -> Result<Vec<Automation>, String> {
    blocking("load_automations", || Ok(load_automations())).await
}

#[tauri::command]
async fn store_automations(list: Vec<Automation>) -> Result<Vec<Automation>, String> {
    blocking("store_automations", move || {
        save_automations(&list)?;
        Ok(list)
    })
    .await
}

#[derive(Serialize)]
struct GitInfo {
    current: String,
    branches: Vec<String>,
}

#[tauri::command]
async fn git_info(cwd: String) -> Result<GitInfo, String> {
    blocking("git_info", move || {
        let fallback = || GitInfo {
            current: "main".into(),
            branches: vec!["main".into()],
        };
        if !Path::new(&cwd).is_dir() {
            return Ok(fallback());
        }
        let run = |args: &[&str]| -> Option<String> {
            let mut command = std::process::Command::new("git");
            command.args(args).current_dir(&cwd);
            let output = process::output_with_timeout(command, Duration::from_secs(4))?;
            if !output.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        };
        let Some(current) = run(&["rev-parse", "--abbrev-ref", "HEAD"]) else {
            return Ok(fallback());
        };
        let branches = run(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .map(|text| {
                text.lines()
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .take(200)
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| vec![current.clone()]);
        Ok(GitInfo { current, branches })
    })
    .await
}

#[derive(Serialize)]
struct DiagnosticsInfo {
    log_path: String,
    log_dir: String,
    version: &'static str,
    os: &'static str,
    arch: &'static str,
}

#[tauri::command]
async fn diagnostics_info() -> Result<DiagnosticsInfo, String> {
    let path = diag::log_path();
    Ok(DiagnosticsInfo {
        log_dir: path
            .parent()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default(),
        log_path: path.display().to_string(),
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    })
}

#[tauri::command]
async fn read_logs() -> Result<String, String> {
    blocking("read_logs", || {
        Ok(format!(
            "Grok Desktop {} · {} {}\n{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            diag::tail()
        ))
    })
    .await
}

#[tauri::command]
async fn open_log_folder() -> Result<(), String> {
    blocking("open_log_folder", || {
        let dir = grok_providers::grok_home();
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program)
            .arg(&dir)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("{program} {}: {error}", dir.display()))
    })
    .await
}

#[tauri::command]
async fn log_frontend(level: String, message: String) -> Result<(), String> {
    let level = match level.as_str() {
        "error" => "ERROR",
        "warn" => "WARN",
        _ => "INFO",
    };
    let message: String = message.chars().take(4000).collect();
    diag::write(level, &format!("ui: {message}"));
    Ok(())
}

pub fn run() {
    diag::init();
    let client = match ProviderClient::new() {
        Ok(client) => client,
        Err(error) => {
            diag::error(&format!("HTTP client failed to start: {error}"));
            panic!("http client: {error}");
        }
    };
    let result = tauri::Builder::default()
        .manage(AppState {
            client,
            secrets: SecretStore::open(),
            cancel: Mutex::new(None),
            approvals: Mutex::new(HashMap::new()),
            turn: tokio::sync::Mutex::new(()),
            mcp: tokio::sync::Mutex::new(None),
            models_refresh_started: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            list_chats,
            save_desktop_settings,
            save_providers,
            set_secret,
            clear_secret,
            create_chat,
            open_chat,
            remove_chat,
            rename_chat,
            branch_chat,
            pick_project_folder,
            check_ready,
            send_prompt,
            cancel_prompt,
            resolve_approval,
            test_provider,
            set_chat_model,
            chat_options,
            set_chat_options,
            fetch_models,
            open_classic,
            account::grok_account,
            account::grok_login,
            account::grok_login_cancel,
            account::grok_logout,
            account::grok_refresh,
            account::open_external_url,
            render_diagram,
            project_media,
            load_automation_list,
            store_automations,
            git_info,
            diagnostics_info,
            read_logs,
            open_log_folder,
            log_frontend,
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        diag::error(&format!("window failed to start: {error}"));
        panic!("error while running grok desktop: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xai() -> ProviderConfig {
        grok_providers::builtin_presets()
            .iter()
            .find(|preset| preset.id == "xai")
            .unwrap()
            .to_config(true)
    }

    #[test]
    fn a_grok_session_goes_to_the_cli_proxy_and_an_api_key_does_not() {
        let mut session = xai();
        route_xai_session(&mut session, XaiSecretSource::Session);
        assert_eq!(
            session.base_url,
            xai_grok_login::CLI_CHAT_PROXY_BASE_URL_DEFAULT
        );
        assert_eq!(
            session
                .extra_headers
                .get("X-XAI-Token-Auth")
                .map(String::as_str),
            Some("xai-grok-cli")
        );
        let mut keyed = xai();
        route_xai_session(&mut keyed, XaiSecretSource::Settings);
        assert_eq!(keyed.base_url, XAI_API_BASE_URL);
        let mut relay = xai();
        relay.base_url = "https://relay.example/v1".into();
        route_xai_session(&mut relay, XaiSecretSource::Session);
        assert_eq!(relay.base_url, "https://relay.example/v1");
    }

    #[test]
    fn a_disabled_or_missing_provider_explains_itself() {
        let settings = DesktopSettings {
            lang: "en".into(),
            ..DesktopSettings::default()
        };
        let mut store = grok_providers::default_store();
        let error = pick_provider(&store, &settings, "nope").unwrap_err();
        assert!(error.contains("No model provider"), "{error}");
        for provider in &mut store.providers {
            provider.enabled = false;
        }
        let error = pick_provider(&store, &settings, "xai").unwrap_err();
        assert!(error.contains("turned off"), "{error}");
    }
}
