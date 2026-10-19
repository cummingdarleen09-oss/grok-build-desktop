use std::path::{Path, PathBuf};
use std::time::Duration;

use grok_providers::ToolDef;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

pub fn tool_defs(lang: &str, disabled: &[String]) -> Vec<ToolDef> {
    let zh = lang.to_ascii_lowercase().starts_with("zh");
    let all = [
        (
            "read_file",
            if zh {
                "读取文本文件。path 相对工作目录。可用 offset 和 limit 指定行范围。"
            } else {
                "Read a text file. path is relative to the working directory. Optional offset and limit select a line range."
            },
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "offset": {"type": "integer", "description": "1-based start line"},
                    "limit": {"type": "integer", "description": "max lines"}
                },
                "required": ["path"]
            }),
        ),
        (
            "write_file",
            if zh {
                "创建或覆盖文件。适合新建文件；修改已有文件优先用 search_replace。"
            } else {
                "Create or overwrite a file. Prefer search_replace when editing an existing file."
            },
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "contents": {"type": "string"}
                },
                "required": ["path", "contents"]
            }),
        ),
        (
            "search_replace",
            if zh {
                "把文件中唯一的一段 old 文本替换为 new。old 必须完全匹配且只出现一次。"
            } else {
                "Replace one unique old string with new. old must match exactly once."
            },
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old": {"type": "string"},
                    "new": {"type": "string"}
                },
                "required": ["path", "old", "new"]
            }),
        ),
        (
            "grep",
            if zh {
                "用正则在工作目录里搜索文本，返回路径、行号和内容。"
            } else {
                "Search the working directory with a regular expression. Returns path, line, and text."
            },
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string", "description": "directory or file, default cwd"}
                },
                "required": ["pattern"]
            }),
        ),
        (
            "list_dir",
            if zh {
                "列出目录中的文件和子目录。"
            } else {
                "List files and subdirectories."
            },
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": []
            }),
        ),
        (
            "run_terminal",
            if zh {
                "在工作目录执行一条 shell 命令，返回 stdout 和 stderr。"
            } else {
                "Run one shell command in the working directory and return stdout and stderr."
            },
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }),
        ),
    ];
    all.into_iter()
        .filter(|(name, _, _)| !disabled.iter().any(|item| item == name))
        .map(|(name, description, parameters)| ToolDef {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        })
        .collect()
}

pub fn needs_approval(name: &str) -> bool {
    matches!(name, "write_file" | "search_replace" | "run_terminal")
}

pub async fn execute_tool(name: &str, arguments: &str, cwd: &Path) -> ToolOutput {
    let value: Value = match serde_json::from_str(arguments) {
        Ok(value) => value,
        Err(error) => {
            return ToolOutput {
                content: format!("arguments are not JSON: {error}"),
                is_error: true,
            };
        }
    };
    match name {
        "read_file" => read_file(cwd, &value),
        "write_file" => write_file(cwd, &value),
        "search_replace" => search_replace(cwd, &value),
        "grep" => grep(cwd, &value),
        "list_dir" => list_dir(cwd, &value),
        "run_terminal" => run_terminal(cwd, &value).await,
        other => ToolOutput {
            content: format!("unknown tool {other}"),
            is_error: true,
        },
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn resolve(cwd: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    Ok(joined)
}

fn read_file(cwd: &Path, value: &Value) -> ToolOutput {
    let Some(raw) = field(value, "path") else {
        return err("path is required");
    };
    let path = match resolve(cwd, raw) {
        Ok(path) => path,
        Err(error) => return err(&error),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => return err(&format!("{}: {error}", path.display())),
    };
    let offset = value
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = value.get("limit").and_then(Value::as_u64).unwrap_or(400) as usize;
    let lines: Vec<&str> = text.lines().collect();
    let start = offset.saturating_sub(1).min(lines.len());
    let end = (start + limit).min(lines.len());
    let mut body = String::new();
    for (index, line) in lines[start..end].iter().enumerate() {
        body.push_str(&format!("{:>5} | {line}\n", start + index + 1));
    }
    if body.len() > 200_000 {
        body.truncate(200_000);
        body.push_str("\n…truncated");
    }
    if body.is_empty() {
        body = "(empty)".into();
    }
    ToolOutput {
        content: body,
        is_error: false,
    }
}

fn write_file(cwd: &Path, value: &Value) -> ToolOutput {
    let Some(raw) = field(value, "path") else {
        return err("path is required");
    };
    let Some(contents) = field(value, "contents") else {
        return err("contents is required");
    };
    let path = match resolve(cwd, raw) {
        Ok(path) => path,
        Err(error) => return err(&error),
    };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return err(&error.to_string());
        }
    }
    match std::fs::write(&path, contents) {
        Ok(()) => ToolOutput {
            content: format!("wrote {} ({} bytes)", path.display(), contents.len()),
            is_error: false,
        },
        Err(error) => err(&format!("{}: {error}", path.display())),
    }
}

fn search_replace(cwd: &Path, value: &Value) -> ToolOutput {
    let Some(raw) = field(value, "path") else {
        return err("path is required");
    };
    let Some(old) = field(value, "old") else {
        return err("old is required");
    };
    let Some(new) = field(value, "new") else {
        return err("new is required");
    };
    if old.is_empty() {
        return err("old must not be empty");
    }
    let path = match resolve(cwd, raw) {
        Ok(path) => path,
        Err(error) => return err(&error),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => return err(&format!("{}: {error}", path.display())),
    };
    let count = text.matches(old).count();
    if count == 0 {
        return err(&format!("old string not found in {}", path.display()));
    }
    if count > 1 {
        return err(&format!(
            "old string appears {count} times in {}; include more context so it is unique",
            path.display()
        ));
    }
    let updated = text.replacen(old, new, 1);
    if let Err(error) = std::fs::write(&path, &updated) {
        return err(&error.to_string());
    }
    ToolOutput {
        content: format!("updated {}", path.display()),
        is_error: false,
    }
}

fn grep(cwd: &Path, value: &Value) -> ToolOutput {
    let Some(pattern) = field(value, "pattern") else {
        return err("pattern is required");
    };
    let regex = match regex::Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => return err(&format!("invalid regex: {error}")),
    };
    let root = match field(value, "path") {
        Some(raw) => match resolve(cwd, raw) {
            Ok(path) => path,
            Err(error) => return err(&error),
        },
        None => cwd.to_path_buf(),
    };
    let mut hits = Vec::new();
    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .max_depth(8)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | "dist" | ".grok"
            )
        });
    for entry in walker.filter_map(Result::ok) {
        if hits.len() >= 80 {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if entry
            .metadata()
            .map(|meta| meta.len() > 1_000_000)
            .unwrap_or(true)
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if regex.is_match(line) {
                hits.push(format!("{}:{}: {line}", path.display(), index + 1));
                if hits.len() >= 80 {
                    break;
                }
            }
        }
    }
    if hits.is_empty() {
        return ToolOutput {
            content: "no matches".into(),
            is_error: false,
        };
    }
    ToolOutput {
        content: hits.join("\n"),
        is_error: false,
    }
}

fn list_dir(cwd: &Path, value: &Value) -> ToolOutput {
    let root = match field(value, "path") {
        Some(raw) => match resolve(cwd, raw) {
            Ok(path) => path,
            Err(error) => return err(&error),
        },
        None => cwd.to_path_buf(),
    };
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) => return err(&format!("{}: {error}", root.display())),
    };
    let mut names = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let mut name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            name.push('/');
        }
        names.push(name);
        if names.len() >= 200 {
            break;
        }
    }
    names.sort();
    ToolOutput {
        content: if names.is_empty() {
            "(empty directory)".into()
        } else {
            names.join("\n")
        },
        is_error: false,
    }
}

async fn run_terminal(cwd: &Path, value: &Value) -> ToolOutput {
    let Some(command) = field(value, "command") else {
        return err("command is required");
    };
    if command.trim().is_empty() {
        return err("command is empty");
    }
    let spawn = if cfg!(windows) {
        tokio::process::Command::new("cmd")
            .args(["/C", command])
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
    } else {
        tokio::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
    };
    let child = match spawn {
        Ok(child) => child,
        Err(error) => return err(&format!("failed to start shell: {error}")),
    };
    let finished = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output()).await;
    let output = match finished {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return err(&error.to_string()),
        Err(_) => return err("command timed out after 120s"),
    };
    let mut text = String::new();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        text.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr);
    }
    text.push_str(&format!("\n[exit {}]", output.status.code().unwrap_or(-1)));
    if text.chars().count() > 40_000 {
        text = text.chars().take(40_000).collect();
        text.push_str("\n…truncated");
    }
    ToolOutput {
        content: text,
        is_error: !output.status.success(),
    }
}

fn err(message: &str) -> ToolOutput {
    ToolOutput {
        content: message.to_string(),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_replace_requires_a_unique_match() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        std::fs::write(&file, "alpha beta alpha\n").unwrap();
        let ambiguous = search_replace(
            dir.path(),
            &json!({"path": "note.txt", "old": "alpha", "new": "A"}),
        );
        assert!(ambiguous.is_error);
        let updated = search_replace(
            dir.path(),
            &json!({"path": "note.txt", "old": "beta", "new": "BETA"}),
        );
        assert!(!updated.is_error, "{}", updated.content);
        assert_eq!(std::fs::read_to_string(file).unwrap(), "alpha BETA alpha\n");
    }

    #[test]
    fn grep_finds_a_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello grok\n").unwrap();
        let found = grep(dir.path(), &json!({"pattern": "grok"}));
        assert!(!found.is_error);
        assert!(found.content.contains("hello grok"), "{}", found.content);
    }
}
