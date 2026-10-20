//! In-app Official Grok sign-in.
//!
//! Uses the same `xai-grok-login` loopback OAuth flow and client id as the CLI.
//! The session is written to the shared `auth.json`.
//! The webview only receives a redacted profile.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use grok_providers::LoginProfile;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};
use xai_grok_login::flow::run_auth_flow_interactive;
use xai_grok_login::{
    AuthManager, AuthUrlInfo, CLI_CHAT_PROXY_BASE_URL_DEFAULT, GrokComConfig,
    LoginTransportOverride, perform_logout, try_ensure_fresh_auth,
};

struct LoginSlot {
    busy: AtomicBool,
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

fn slot() -> &'static LoginSlot {
    static SLOT: OnceLock<LoginSlot> = OnceLock::new();
    SLOT.get_or_init(|| LoginSlot {
        busy: AtomicBool::new(false),
        cancel: Mutex::new(None),
    })
}

fn finish_login() {
    if let Ok(mut cancel) = slot().cancel.lock() {
        *cancel = None;
    }
    slot().busy.store(false, Ordering::SeqCst);
}

fn proxy_url() -> String {
    CLI_CHAT_PROXY_BASE_URL_DEFAULT.to_string()
}

fn auth_manager(config: GrokComConfig) -> AuthManager {
    AuthManager::new_with_proxy_base_url(&xai_grok_config::grok_home(), config, proxy_url())
}

fn redact_login_error(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Sign-in failed.");
    let lower = line.to_ascii_lowercase();
    if lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("eyj")
        || lower.contains("code=")
    {
        return "Sign-in failed.".into();
    }
    let mut end = line.len();
    let mut count = 0;
    for (index, _) in line.char_indices() {
        if count == 240 {
            end = index;
            break;
        }
        count += 1;
    }
    if end < line.len() {
        format!("{}…", &line[..end])
    } else {
        line.to_string()
    }
}

async fn run_login(
    url_tx: oneshot::Sender<AuthUrlInfo>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let config = GrokComConfig::default();
    let manager = Arc::new(auth_manager(config.clone()));
    let url_slot = Rc::new(RefCell::new(Some(url_tx)));
    let (code_tx, code_rx) = mpsc::channel::<String>(1);
    let login = run_auth_flow_interactive(
        &manager,
        &config,
        None,
        None,
        Some(url_slot),
        Some(code_rx),
        LoginTransportOverride::ForceLoopback,
    );
    tokio::pin!(login);
    let result = tokio::select! {
        biased;
        _ = &mut cancel_rx => Err("Sign-in cancelled.".to_string()),
        result = &mut login => result.map(|_| ()).map_err(|err| redact_login_error(&err.to_string())),
    };
    drop(code_tx);
    result
}

#[tauri::command]
pub async fn grok_account() -> LoginProfile {
    tauri::async_runtime::spawn_blocking(grok_providers::read_login_profile)
        .await
        .unwrap_or_else(|_| grok_providers::read_login_profile())
}

/// Refreshes an expired Grok session before a request. Returns false when the
/// session could not be refreshed within the time limit.
pub async fn refresh_session() -> bool {
    if slot().busy.load(Ordering::SeqCst) {
        return false;
    }
    let config = GrokComConfig::default();
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            try_ensure_fresh_auth(&config, proxy_url()),
        )
        .await,
        Ok(Some(_))
    )
}

#[tauri::command]
pub async fn grok_login(app: AppHandle) -> Result<LoginProfile, String> {
    if slot().busy.swap(true, Ordering::SeqCst) {
        return Err("Sign-in is already in progress.".into());
    }
    let (cancel_tx, cancel_rx) = oneshot::channel();
    if let Ok(mut cancel) = slot().cancel.lock() {
        *cancel = Some(cancel_tx);
    }
    let (url_tx, url_rx) = oneshot::channel();
    let (done_tx, done_rx) = oneshot::channel::<Result<(), String>>();
    let spawned = std::thread::Builder::new()
        .name("grok-oauth".into())
        .spawn(move || {
            struct Clear;
            impl Drop for Clear {
                fn drop(&mut self) {
                    finish_login();
                }
            }
            let _clear = Clear;
            let outcome = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(run_login(url_tx, cancel_rx)),
                Err(error) => Err(error.to_string()),
            };
            let _ = done_tx.send(outcome);
        });
    if let Err(error) = spawned {
        finish_login();
        return Err(error.to_string());
    }
    tokio::spawn(async move {
        if let Ok(info) = url_rx.await {
            let _ = app.emit("grok-login-url", info.url);
        }
    });
    match done_rx.await {
        Ok(result) => {
            result?;
            Ok(grok_providers::read_login_profile())
        }
        Err(_) => Err("Sign-in stopped.".into()),
    }
}

#[tauri::command]
pub fn grok_login_cancel() {
    if let Ok(mut cancel) = slot().cancel.lock() {
        if let Some(sender) = cancel.take() {
            let _ = sender.send(());
        }
    }
}

#[tauri::command]
pub async fn grok_logout() -> Result<LoginProfile, String> {
    if slot().busy.load(Ordering::SeqCst) {
        return Err("Sign-in is still running.".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let manager = auth_manager(GrokComConfig::default());
        perform_logout(&manager, None, || {}).map_err(|error| error.to_string())?;
        Ok(grok_providers::read_login_profile())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn grok_refresh() -> Result<LoginProfile, String> {
    if slot().busy.load(Ordering::SeqCst) {
        return Err("Sign-in is still running.".into());
    }
    let config = GrokComConfig::default();
    match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        try_ensure_fresh_auth(&config, proxy_url()),
    )
    .await
    {
        Ok(Some(_)) => Ok(grok_providers::read_login_profile()),
        Ok(None) => Err("Could not refresh the Grok session. Sign in again.".into()),
        Err(_) => {
            Err("Refreshing the Grok session timed out. Check the network and try again.".into())
        }
    }
}

#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only http(s) links can be opened.".into());
    }
    webbrowser::open(url).map_err(|error| error.to_string())
}
