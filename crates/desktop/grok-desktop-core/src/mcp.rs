//! Stdio MCP client for servers declared in `config.toml`.
//!
//! HTTP and SSE transports stay on the standalone CLI. A desktop turn connects
//! to each enabled stdio server, lists its tools, and drops the process when
//! the turn ends.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use grok_providers::ToolDef;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

use crate::tools::ToolOutput;

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
    #[serde(default)]
    url: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

struct LiveServer {
    name: String,
    /// Kept so `kill_on_drop` tears the server down with the turn.
    #[allow(dead_code)]
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

pub struct McpPool {
    servers: Vec<LiveServer>,
}

impl McpPool {
    pub fn empty() -> Self {
        Self {
            servers: Vec::new(),
        }
    }

    pub async fn connect() -> Self {
        let path = grok_providers::grok_home().join("config.toml");
        Self::connect_file(&path).await
    }

    pub async fn connect_file(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::empty();
        };
        let Ok(file) = toml::from_str::<McpFile>(&text) else {
            return Self::empty();
        };
        let mut servers = Vec::new();
        for (name, spec) in file.mcp_servers {
            if !spec.enabled || file.disabled_mcp_servers.iter().any(|item| item == &name) {
                continue;
            }
            let _transport = spec.url.as_str();
            if spec.command.trim().is_empty() {
                continue;
            }
            if let Some(live) = LiveServer::start(name, spec).await {
                servers.push(live);
            }
        }
        Self { servers }
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

impl LiveServer {
    async fn start(name: String, spec: McpServer) -> Option<Self> {
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .envs(&spec.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let mut live = Self {
            name,
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
            tools: Vec::new(),
        };
        if live.handshake().await.is_err() {
            let _ = live.child.kill().await;
            return None;
        }
        Some(live)
    }

    async fn handshake(&mut self) -> Result<(), String> {
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "grok-desktop", "version": "0.2.0"}
                }),
            )
            .await?;
        if init.get("error").is_some() {
            return Err("mcp initialize failed".into());
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
        let deadline = Duration::from_secs(8);
        tokio::time::timeout(deadline, async {
            loop {
                let message = read_message(&mut self.reader).await?;
                if message.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(message);
                }
            }
        })
        .await
        .map_err(|_| "mcp request timed out".to_string())?
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn send(&mut self, value: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        self.stdin
            .write_all(&body)
            .await
            .map_err(|error| error.to_string())?;
        self.stdin.flush().await.map_err(|error| error.to_string())
    }
}

async fn read_message(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<Value, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("mcp server closed stdout".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }
    let len = content_length.ok_or_else(|| "mcp frame is missing Content-Length".to_string())?;
    let mut buf = vec![0_u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&buf).map_err(|error| error.to_string())
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

    #[tokio::test]
    async fn stdio_server_lists_and_calls_a_tool() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.py");
        std::fs::write(
            &script,
            r#"
import json, sys
def read_message():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if line in (b"",):
            raise SystemExit(0)
        if line in (b"\r\n", b"\n"):
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    body = sys.stdin.buffer.read(length)
    return json.loads(body)
def write_message(value):
    data = json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    sys.stdout.buffer.flush()
while True:
    msg = read_message()
    method = msg.get("method")
    if method == "initialize":
        write_message({"jsonrpc":"2.0","id":msg["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"demo","version":"0"}}})
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        write_message({"jsonrpc":"2.0","id":msg["id"],"result":{"tools":[{"name":"echo","description":"Echo text","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]}})
    elif method == "tools/call":
        text = (msg.get("params") or {}).get("arguments", {}).get("text", "")
        write_message({"jsonrpc":"2.0","id":msg["id"],"result":{"content":[{"type":"text","text":"echo:"+text}],"isError":False}})
"#,
        )
        .unwrap();
        let config = dir.path().join("config.toml");
        let script_path = script.display().to_string().replace('\\', "\\\\");
        std::fs::write(
            &config,
            format!(
                "[mcp_servers.demo]\ncommand = \"python3\"\nargs = [\"{script_path}\"]\n\n[mcp_servers.remote]\nurl = \"https://example.invalid/mcp\"\n"
            ),
        )
        .unwrap();
        let mut pool = McpPool::connect_file(&config).await;
        let names: Vec<_> = pool.tool_defs().into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, vec!["mcp__demo__echo".to_string()]);
        let output = pool
            .call("mcp__demo__echo", r#"{"text":"hi"}"#)
            .await
            .expect("tool routed");
        assert!(!output.is_error);
        assert_eq!(output.content, "echo:hi");
    }
}
