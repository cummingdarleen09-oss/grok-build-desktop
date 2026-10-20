//! Diagnostics log at `~/.grok/desktop.log` (or `$GROK_HOME/desktop.log`).
//!
//! Lines carry timings and errors so a bug report can include them. Keys and
//! tokens are never written: provider errors are redacted before they get
//! here, and [`scrub`] masks anything shaped like a bearer token.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;

static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
static STARTED: OnceLock<Instant> = OnceLock::new();

pub fn log_path() -> PathBuf {
    grok_providers::grok_home().join("desktop.log")
}

fn rotated_path() -> PathBuf {
    grok_providers::grok_home().join("desktop.log.1")
}

pub fn init() {
    STARTED.get_or_init(Instant::now);
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, rotated_path());
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    let _ = FILE.set(Mutex::new(file));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_default();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        write("PANIC", &format!("{payload} at {location}"));
        previous(info);
    }));
    write(
        "INFO",
        &format!(
            "Grok Desktop {} starting on {} {} (grok home {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            grok_providers::grok_home().display()
        ),
    );
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn warn(message: &str) {
    write("WARN", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

pub fn write(level: &str, message: &str) {
    let uptime = STARTED
        .get()
        .map(|started| started.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let line = format!(
        "{} +{uptime:.3}s {level:<5} {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        scrub(message).replace('\n', " ⏎ ")
    );
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    let Some(slot) = FILE.get() else {
        return;
    };
    let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(file) = guard.as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Mask long token-like words (API keys, JWTs, OAuth codes).
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, word) in text.split(' ').enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let core = word.trim_matches(|ch: char| "\"',;()[]{}".contains(ch));
        let tokenish = core.len() >= 24
            && core
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-_.=+/".contains(ch))
            && core.chars().any(|ch| ch.is_ascii_digit())
            && !core.starts_with("http")
            && !core.contains('/')
            && !is_uuid(core);
        let keyish = ["sk-", "xai-", "eyJ"]
            .iter()
            .any(|prefix| core.starts_with(prefix))
            && core.len() >= 12;
        if tokenish || keyish {
            out.push_str(&word.replace(core, "•••"));
        } else {
            out.push_str(word);
        }
    }
    out
}

fn is_uuid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

/// The end of the current log, plus the rotated file when the current one is short.
pub fn tail() -> String {
    let mut text = String::new();
    let current = read_tail(&log_path(), TAIL_BYTES);
    if (current.len() as u64) < TAIL_BYTES / 2 {
        let older = read_tail(&rotated_path(), TAIL_BYTES / 2);
        if !older.is_empty() {
            text.push_str(&older);
            text.push_str("\n--- desktop.log ---\n");
        }
    }
    text.push_str(&current);
    text
}

fn read_tail(path: &std::path::Path, max: u64) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    if len > max {
        let _ = file.seek(SeekFrom::Start(len - max));
    }
    let mut buf = Vec::new();
    let _ = file.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    if len > max {
        match text.find('\n') {
            Some(index) => text[index + 1..].to_string(),
            None => text,
        }
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn keys_and_tokens_are_masked() {
        let line =
            scrub("HTTP 401: invalid key sk-proj-abcdefghijklmnop12345 for eyJhbGciOiJIUzI1NiJ9.x");
        assert!(!line.contains("abcdefghijklmnop"), "{line}");
        assert!(!line.contains("eyJhbGci"), "{line}");
        assert!(line.contains("HTTP 401"));
        let url = scrub("POST https://api.deepseek.com/chat/completions took 812 ms");
        assert!(
            url.contains("https://api.deepseek.com/chat/completions"),
            "{url}"
        );
        let id = scrub("session 3f2b8c1e-9a4d-4e2b-8f1a-0c2d4e6f8a9b saved");
        assert!(id.contains("3f2b8c1e-9a4d-4e2b-8f1a-0c2d4e6f8a9b"), "{id}");
    }
}
