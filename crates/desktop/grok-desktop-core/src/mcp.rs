//! Stdio MCP client for servers declared in `config.toml`.
//!
//! HTTP and SSE transports stay on the standalone CLI. The desktop connects to
//! each enabled stdio server once, keeps the pool across turns, and reconnects
//! when `config.toml` changes or a server exits.
//!
//! Messages are newline-delimited JSON-RPC, as the MCP stdio transport
//! specifies. Replies framed with `Content-Length` headers are still read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use grok_providers::ToolDef;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

use crate::tools::ToolOutput;

/// Upper bound for starting one server and listing its tools. `npx -y` may
/// download the package on first use, so this is generous.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, serde::Deserialize)]
struct McpFile {
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServer>,
    #[serde(default)]
    disabled_mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpServer {
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

struct LiveServer {
    name: String,
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
    tools: Vec<ListedTool>,
}

struct ListedTool {
    exposed: String,
    raw: String,
    def: ToolDef,
}

/// Identifies one version of `config.toml`, so a cached pool can tell when to reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpFingerprint {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

pub struct McpPool {
    servers: Vec<LiveServer>,
    fingerprint: Option<McpFingerprint>,
    /// Servers that were configured but did not start, with the reason.
    pub failures: Vec<(String, String)>,
}

impl McpPool {
    pub fn empty() -> Self {
        Self {
            servers: Vec::new(),
            fingerprint: None,
            failures: Vec::new(),
        }
    }

    pub fn config_path() -> PathBuf {
        grok_providers::grok_home().join("config.toml")
    }

    pub fn fingerprint_of(path: &Path) -> Option<McpFingerprint> {
        let meta = std::fs::metadata(path).ok()?;
        Some(McpFingerprint {
            path: path.to_path_buf(),
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }

    /// Names of the stdio servers `path` would start. Reading this is cheap,
    /// so callers can skip a "connecting" notice when there is nothing to start.
    pub fn configured_servers(path: &Path) -> Vec<String> {
        read_specs(path).into_iter().map(|(name, _)| name).collect()
    }

    pub async fn connect() -> Self {
        Self::connect_file(&Self::config_path()).await
    }

    pub async fn connect_file(path: &Path) -> Self {
        let fingerprint = Self::fingerprint_of(path);
        let specs = read_specs(path);
        let started = futures_util::future::join_all(specs.into_iter().map(|(name, spec)| {
            let label = name.clone();
            async move {
                match tokio::time::timeout(CONNECT_TIMEOUT, LiveServer::start(name, spec)).await {
                    Ok(result) => (label, result),
                    Err(_) => (
                        label,
                        Err(format!(
                            "did not answer within {}s",
                            CONNECT_TIMEOUT.as_secs()
                        )),
                    ),
                }
            }
        }))
        .await;
        let mut servers = Vec::new();
        let mut failures = Vec::new();
        for (name, result) in started {
            match result {
                Ok(live) => servers.push(live),
                Err(error) => failures.push((name, error)),
            }
        }
        Self {
            servers,
            fingerprint,
            failures,
        }
    }

    /// True while the pool matches `config.toml` and every server is still running.
    pub fn is_current(&mut self, path: &Path) -> bool {
        if self.fingerprint != Self::fingerprint_of(path) {
            return false;
        }
        self.servers
            .iter_mut()
            .all(|server| matches!(server.child.try_wait(), Ok(None)))
    }

    pub fn tool_defs(&self) -> Vec<ToolDef> {
        self.servers
            .iter()
            .flat_map(|server| server.tools.iter().map(|tool| tool.def.clone()))
            .collect()
    }

    pub async fn call(&mut self, name: &str, arguments: &str) -> Option<ToolOutput> {
        let server = self
            .servers
            .iter_mut()
            .find(|server| server.tools.iter().any(|tool| tool.exposed == name))?;
        let raw = server
            .tools
            .iter()
            .find(|tool| tool.exposed == name)
            .map(|tool| tool.raw.clone())?;
        Some(server.call(&raw, arguments).await)
    }
}

fn read_specs(path: &Path) -> Vec<(String, McpServer)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(file) = toml::from_str::<McpFile>(&text) else {
        return Vec::new();
    };
    file.mcp_servers
        .into_iter()
        .filter(|(name, spec)| {
            spec.enabled
                && !file.disabled_mcp_servers.iter().any(|item| item == name)
                && !spec.command.trim().is_empty()
        })
        .collect()
}

/// `npx`, `uvx`, and other launchers are `.cmd` shims on Windows, which
/// `CreateProcess` cannot start directly. Run those through `cmd /C`.
fn server_command(spec: &McpServer) -> Command {
    #[cfg(windows)]
    {
        let program = Path::new(spec.command.trim());
        let has_exe = program
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
        if !has_exe {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(spec.command.trim()).args(&spec.args);
            return command;
        }
    }
    let mut command = Command::new(spec.command.trim());
    command.args(&spec.args);
    command
}

impl LiveServer {
    async fn start(name: String, spec: McpServer) -> Result<Self, String> {
        let mut command = server_command(&spec);
        command
            .envs(&spec.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        crate::process::hide_window_tokio(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("{}: {error}", spec.command))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut live = Self {
            name,
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
            tools: Vec::new(),
        };
        if let Err(error) = live.handshake().await {
            let _ = live.child.kill().await;
            return Err(error);
        }
        Ok(live)
    }

    async fn handshake(&mut self) -> Result<(), String> {
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "grok-desktop", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        if let Some(error) = init.get("error") {
            return Err(format!("initialize failed: {error}"));
        }
        self.notify("notifications/initialized", json!({})).await?;
        let listed = self.request("tools/list", json!({})).await?;
        let tools = listed
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for tool in tools {
            let Some(raw_name) = tool.get("name").and_then(Value::as_str) else {
                continue;
            };
            let exposed = format!("mcp__{}__{}", sanitize(&self.name), sanitize(raw_name));
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("MCP tool");
            let parameters = tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            self.tools.push(ListedTool {
                exposed: exposed.clone(),
                raw: raw_name.to_string(),
                def: ToolDef {
                    name: exposed,
                    description: format!(
                        "MCP tool `{raw_name}` from server `{}`. {description}",
                        self.name
                    ),
                    parameters,
                },
            });
        }
        Ok(())
    }

    async fn call(&mut self, tool: &str, arguments: &str) -> ToolOutput {
        let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
        let response = match self
            .request("tools/call", json!({"name": tool, "arguments": args}))
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return ToolOutput {
                    content: error,
                    is_error: true,
                };
            }
        };
        if let Some(error) = response.get("error") {
            return ToolOutput {
                content: error.to_string(),
                is_error: true,
            };
        }
        let result = response.get("result").cloned().unwrap_or(Value::Null);
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let content = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| result.to_string());
        ToolOutput { content, is_error }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            loop {
                let message = read_message(&mut self.reader).await?;
                if message.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(message);
                }
            }
        })
        .await
        .map_err(|_| format!("{method} timed out"))?
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn send(&mut self, value: &Value) -> Result<(), String> {
        let mut body = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        body.push(b'\n');
        self.stdin
            .write_all(&body)
            .await
            .map_err(|error| error.to_string())?;
        self.stdin.flush().await.map_err(|error| error.to_string())
    }
}

/// Next JSON-RPC message from the server. Lines that are not JSON (some
/// servers print banners to stdout) are skipped.
async fn read_message<R>(reader: &mut BufReader<R>) -> Result<Value, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("server closed stdout".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            let len = rest
                .trim()
                .parse::<usize>()
                .map_err(|_| "bad Content-Length header".to_string())?;
            loop {
                let mut header = String::new();
                let read = reader
                    .read_line(&mut header)
                    .await
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    return Err("server closed stdout".into());
                }
                if header.trim().is_empty() {
                    break;
                }
            }
            let mut buf = vec![0_u8; len];
            reader
                .read_exact(&mut buf)
                .await
                .map_err(|error| error.to_string())?;
            return serde_json::from_slice(&buf).map_err(|error| error.to_string());
        }
        if trimmed.starts_with('{')
            && let Ok(value) = serde_json::from_str::<Value>(trimmed)
        {
            return Ok(value);
        }
    }
}

fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "tool".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = r#"
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    def reply(result):
        sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":msg["id"],"result":result}) + "\n")
        sys.stdout.flush()
    if method == "initialize":
        print("demo server ready")
        sys.stdout.flush()
        reply({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"demo","version":"0"}})
    elif method == "tools/list":
        reply({"tools":[{"name":"echo","description":"Echo text","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]})
    elif method == "tools/call":
        text = (msg.get("params") or {}).get("arguments", {}).get("text", "")
        reply({"content":[{"type":"text","text":"echo:"+text}],"isError":False})
"#;

    fn python() -> Option<&'static str> {
        ["python3", "python"].into_iter().find(|program| {
            std::process::Command::new(program)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
    }

    #[tokio::test]
    async fn stdio_server_lists_and_calls_a_tool() {
        let Some(python) = python() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.py");
        std::fs::write(&script, SERVER).unwrap();
        let config = dir.path().join("config.toml");
        let script_path = script.display().to_string().replace('\\', "\\\\");
        std::fs::write(
            &config,
            format!(
                "[mcp_servers.demo]\ncommand = \"{python}\"\nargs = [\"{script_path}\"]\n\n[mcp_servers.remote]\nurl = \"https://example.invalid/mcp\"\n\n[mcp_servers.missing]\ncommand = \"definitely-not-installed-mcp\"\n"
            ),
        )
        .unwrap();
        let mut pool = McpPool::connect_file(&config).await;
        let names: Vec<_> = pool.tool_defs().into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, vec!["mcp__demo__echo".to_string()]);
        assert_eq!(pool.failures.len(), 1);
        assert_eq!(pool.failures[0].0, "missing");
        assert!(pool.is_current(&config));
        let output = pool
            .call("mcp__demo__echo", r#"{"text":"hi"}"#)
            .await
            .expect("tool routed");
        assert!(!output.is_error);
        assert_eq!(output.content, "echo:hi");
        std::fs::write(&config, "").unwrap();
        assert!(!pool.is_current(&config));
    }

    #[tokio::test]
    async fn content_length_frames_are_still_read() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let mut reader = BufReader::new(framed.as_bytes());
        let value = read_message(&mut reader).await.unwrap();
        assert_eq!(value["id"], 1);
    }
}
