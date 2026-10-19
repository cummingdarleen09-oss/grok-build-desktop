//! Tauri shell around [`grok_desktop_core`].
//!
//! The window is the desktop workbench (sessions, projects, media, automations).
//! Model traffic goes through [`grok_providers`]. The original CLI still builds
//! as its own binary and is not embedded in this window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use grok_desktop_core::{
    AgentEvent, Automation, DesktopSettings, ExtensionRecord, MediaFile, Session, SessionSummary,
    TurnControl, branch_session, create_session, delete_session, discover_extensions, list_media,
    list_sessions, load_automations, load_classic_session, load_session, load_settings,
    rename_session, render_mermaid, run_turn, save_automations, save_session, save_settings, tr,
};
use grok_providers::{
    ProviderClient, ProviderConfig, ProviderFormat, ProviderStore, SecretLocation, SecretStore,
    Turn, apply_choice, fetch_provider_models, has_login_session, load_or_default_store,
    normalize_store, refresh_xai_models, save_provider_store, sync_cli_config,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    client: ProviderClient,
    secrets: SecretStore,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    approvals: Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    turn: tokio::sync::Mutex<()>,
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
}

#[derive(Serialize, Clone)]
struct AgentEnvelope {
    session_id: String,
    event: AgentEvent,
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

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let mut providers = load_or_default_store();
    if let Some(xai) = providers
        .providers
        .iter()
        .find(|provider| provider.id == "xai")
        .cloned()
    {
        let key = state.secrets.resolve(&xai, None).ok().flatten();
        let choice = refresh_xai_models(&xai.base_url, key.as_deref()).await;
        let before = providers.clone();
        apply_choice(&mut providers, &choice);
        if providers != before {
            let _ = save_provider_store(&providers);
            let _ = sync_cli_config(&providers.providers);
        }
    }
    let secrets = providers
        .providers
        .iter()
        .map(|provider| secret_status(&state.secrets, provider))
        .collect();
    Ok(Bootstrap {
        settings: load_settings(),
        providers,
        sessions: list_sessions()?,
        extensions: discover_extensions(),
        secrets,
        grok_home: grok_providers::grok_home().display().to_string(),
        user_name: user_name(),
        automations: load_automations(),
    })
}

fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Grok".into())
}

#[tauri::command]
fn save_desktop_settings(settings: DesktopSettings) -> Result<(), String> {
    save_settings(&settings)
}

#[tauri::command]
fn save_providers(store: ProviderStore) -> Result<ProviderStore, String> {
    for provider in &store.providers {
        provider.validate()?;
    }
    let store = normalize_store(store);
    save_provider_store(&store)?;
    sync_cli_config(&store.providers)?;
    Ok(store)
}

#[tauri::command]
fn set_secret(
    state: State<'_, AppState>,
    id: String,
    secret: String,
) -> Result<&'static str, String> {
    match state.secrets.set(&id, &secret)? {
        SecretLocation::Keyring => Ok("keyring"),
        SecretLocation::File => Ok("file"),
    }
}

#[tauri::command]
fn clear_secret(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.secrets.delete(&id)
}

#[tauri::command]
fn create_chat(
    cwd: String,
    provider_id: String,
    model: String,
    lang: String,
) -> Result<Session, String> {
    let session = create_session(&cwd, &provider_id, &model, &tr(&lang, "new_chat"));
    save_session(&session)?;
    Ok(session)
}

#[tauri::command]
fn open_chat(id: String) -> Result<Session, String> {
    load_session(&id)
}

#[tauri::command]
fn remove_chat(id: String) -> Result<(), String> {
    delete_session(&id)
}

#[tauri::command]
fn rename_chat(id: String, title: String) -> Result<Session, String> {
    rename_session(&id, &title)
}

#[tauri::command]
fn branch_chat(
    session_id: String,
    message_id: String,
    include_message: bool,
    lang: String,
) -> Result<Session, String> {
    let parent = load_session(&session_id)?;
    let title = format!("{} · {}", tr(&lang, "branch_title"), parent.title);
    branch_session(&session_id, &message_id, include_message, &title)
}

#[tauri::command]
fn pick_project_folder() -> Result<String, String> {
    match pick_folder() {
        Some(path) if !path.trim().is_empty() => Ok(path),
        Some(_) | None => Err("cancelled".into()),
    }
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
        return picker_stdout(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Open project'; if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $d.SelectedPath }",
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
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Ok(None);
    }
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    while text.len() > 1 && (text.ends_with('/') || text.ends_with('\\')) {
        text.pop();
    }
    Ok(if text.is_empty() { None } else { Some(text) })
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
    let _turn = state.turn.lock().await;
    let mut session = load_session(&session_id)?;
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        session.model = model;
    }
    let settings = load_settings();
    let store = load_or_default_store();
    let provider_id = if session.provider_id.is_empty() {
        settings.active_provider.clone().unwrap_or_default()
    } else {
        session.provider_id.clone()
    };
    let mut provider = store
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned()
        .ok_or_else(|| tr(&settings.lang, "no_provider"))?;
    if !session.model.trim().is_empty() {
        provider.model = session.model.clone();
    }
    if !provider.enabled {
        return Err(tr(&settings.lang, "no_provider"));
    }
    let api_key = state
        .secrets
        .resolve(&provider, None)?
        .ok_or_else(|| tr(&settings.lang, "missing_key"))?;
    if session.cwd.is_empty() {
        session.cwd = settings.cwd.clone();
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().expect("cancel lock") = Some(cancel.clone());
    let approvals = &state.approvals;
    let session_for_events = session_id.clone();
    let result = run_turn(
        &mut session,
        &provider,
        &api_key,
        &text,
        &settings.lang,
        settings.asks_permission(),
        plan.unwrap_or(false),
        &settings.disabled_tools,
        &state.client,
        &TurnControl { cancel },
        |event| {
            let _ = app.emit(
                "agent-event",
                AgentEnvelope {
                    session_id: session_for_events.clone(),
                    event,
                },
            );
        },
        |id, _name, _arguments| {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            approvals.lock().expect("approvals").insert(id, sender);
            async move { receiver.await.unwrap_or(false) }
        },
    )
    .await;
    save_session(&session)?;
    *state.cancel.lock().expect("cancel lock") = None;
    result?;
    Ok(session)
}

#[tauri::command]
fn cancel_prompt(state: State<'_, AppState>) {
    if let Some(flag) = state.cancel.lock().expect("cancel lock").as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn resolve_approval(state: State<'_, AppState>, id: String, approved: bool) -> Result<(), String> {
    let sender = state
        .approvals
        .lock()
        .expect("approvals")
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
    let api_key = state
        .secrets
        .resolve(&provider, None)?
        .ok_or_else(|| tr(&settings.lang, "missing_key"))?;
    provider.max_tokens = 32;
    let text = state
        .client
        .complete_text(
            &provider,
            &api_key,
            &[Turn::user("Reply with the single word pong.")],
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(text)
}

#[tauri::command]
fn set_chat_model(id: String, provider_id: String, model: String) -> Result<Session, String> {
    let mut session = load_session(&id)?;
    session.provider_id = provider_id;
    session.model = model;
    save_session(&session)?;
    Ok(session)
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
        let store = load_or_default_store();
        match store
            .providers
            .into_iter()
            .find(|provider| provider.id == id)
        {
            Some(provider) => state.secrets.resolve(&provider, None)?.unwrap_or_default(),
            None => String::new(),
        }
    } else {
        api_key
    };
    if key.trim().is_empty() {
        return Err("missing api key".into());
    }
    fetch_provider_models(&base_url, auth, &key).await
}

#[tauri::command]
fn open_classic(id: String) -> Result<Session, String> {
    load_classic_session(&id)
}

#[tauri::command]
fn render_diagram(source: String, dark: bool) -> Result<String, String> {
    render_mermaid(&source, dark)
}

#[tauri::command]
fn project_media(cwd: String) -> Vec<MediaFile> {
    list_media(&cwd)
}

#[tauri::command]
fn load_automation_list() -> Vec<Automation> {
    load_automations()
}

#[tauri::command]
fn store_automations(list: Vec<Automation>) -> Result<Vec<Automation>, String> {
    save_automations(&list)?;
    Ok(list)
}

#[derive(Serialize)]
struct GitInfo {
    current: String,
    branches: Vec<String>,
}

#[tauri::command]
fn git_info(cwd: String) -> GitInfo {
    let run = |args: &[&str]| -> Option<String> {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&cwd)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    };
    let current = run(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "main".into());
    let branches = run(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .map(|text| {
            text.lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec![current.clone()]);
    GitInfo { current, branches }
}

pub fn run() {
    let client = ProviderClient::new().expect("http client");
    tauri::Builder::default()
        .manage(AppState {
            client,
            secrets: SecretStore::open(),
            cancel: Mutex::new(None),
            approvals: Mutex::new(std::collections::HashMap::new()),
            turn: tokio::sync::Mutex::new(()),
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
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
            send_prompt,
            cancel_prompt,
            resolve_approval,
            test_provider,
            set_chat_model,
            fetch_models,
            open_classic,
            render_diagram,
            project_media,
            load_automation_list,
            store_automations,
            git_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running grok desktop");
}
