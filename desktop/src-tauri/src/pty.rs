//! Host the original grok TUI inside the desktop window.
//!
//! Every slash command, plugin, hook, and keyboard shortcut of the CLI keeps
//! working here because this is that binary, on a PTY, with provider API keys
//! injected from the desktop secret store.

use std::io::{Read, Write};
use std::path::PathBuf;

use base64::Engine;
use grok_desktop_core::load_settings;
use grok_providers::{SecretStore, load_or_default_store};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tauri::{AppHandle, Emitter};

pub struct PtySession {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtySession {
    pub fn start(
        app: AppHandle,
        cols: u16,
        rows: u16,
        secrets: &SecretStore,
    ) -> Result<(Self, String), String> {
        let binary = find_grok_binary().ok_or_else(|| {
            "Could not find the grok binary. Build it with `cargo build -p xai-grok-pager-bin` or set GROK_BIN.".to_string()
        })?;
        let system = native_pty_system();
        let pair = system
            .openpty(PtySize {
                rows: rows.max(8),
                cols: cols.max(20),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let mut command = CommandBuilder::new(binary.as_os_str());
        let settings = load_settings();
        if !settings.cwd.is_empty() {
            command.cwd(&settings.cwd);
        }
        inject_provider_keys(&mut command, secrets)?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let app_for_reader = app.clone();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let encoded =
                            base64::engine::general_purpose::STANDARD.encode(&buffer[..count]);
                        let _ = app_for_reader.emit("pty-output", encoded);
                    }
                }
            }
            let _ = app_for_reader.emit("pty-exit", ());
        });
        let session = Self {
            master: pair.master,
            writer,
            child,
        };
        Ok((session, binary.display().to_string()))
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(data)
            .map_err(|error| error.to_string())?;
        self.writer.flush().map_err(|error| error.to_string())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows: rows.max(8),
                cols: cols.max(20),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

fn inject_provider_keys(command: &mut CommandBuilder, secrets: &SecretStore) -> Result<(), String> {
    let store = load_or_default_store();
    for provider in store.providers.iter().filter(|provider| provider.enabled) {
        if let Some(secret) = secrets.get(&provider.id)? {
            if !provider.env_key.is_empty() {
                command.env(&provider.env_key, &secret);
            }
            for fallback in &provider.env_key_fallbacks {
                command.env(fallback, &secret);
            }
        }
    }
    Ok(())
}

pub fn find_grok_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GROK_BIN") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for name in ["grok", "xai-grok-pager", "grok.exe", "xai-grok-pager.exe"] {
                let candidate = parent.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    for name in ["grok", "xai-grok-pager"] {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }
    }
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    for root in roots {
        let mut cursor = Some(root.as_path());
        for _ in 0..6 {
            let Some(dir) = cursor else { break };
            for relative in [
                "target/debug/xai-grok-pager",
                "target/release/xai-grok-pager",
            ] {
                let candidate = dir.join(relative);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            cursor = dir.parent();
        }
    }
    None
}
