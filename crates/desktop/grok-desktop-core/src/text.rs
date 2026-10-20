/// Short user-facing strings. The React UI has the full catalog; these are the
/// messages the agent loop itself has to show.
pub fn tr(lang: &str, key: &str) -> String {
    let zh = lang.to_ascii_lowercase().starts_with("zh");
    let text = match key {
        "missing_key" => {
            if zh {
                "没有可用的 API 密钥。请在设置里保存，或导出对应的环境变量。"
            } else {
                "No API key is available. Save one in Settings, or export the provider environment variable."
            }
        }
        "missing_key_for" => {
            if zh {
                "「{name}」还没有可用的凭据。请在 设置 → 模型供应商 里登录 Grok 或保存 API 密钥。"
            } else {
                "{name} has no credentials yet. Sign in with Grok or save an API key in Settings → Model providers."
            }
        }
        "no_provider" => {
            if zh {
                "还没有可用的模型供应商。请在 设置 → 模型供应商 里登录 Grok，或添加并启用一个供应商。"
            } else {
                "No model provider is available. Sign in with Grok, or add and enable a provider in Settings → Model providers."
            }
        }
        "provider_disabled" => {
            if zh {
                "「{name}」已停用。请在设置里启用它，或在输入框下方换一个模型。"
            } else {
                "{name} is turned off. Enable it in Settings, or pick another model below the composer."
            }
        }
        "busy" => {
            if zh {
                "上一条回复还在进行中。请等待它完成，或点「停止」。"
            } else {
                "A reply is still running. Wait for it to finish, or press Stop."
            }
        }
        "cwd_missing" => {
            if zh {
                "项目文件夹不存在：{path}。请在左侧重新选择项目。"
            } else {
                "The project folder does not exist: {path}. Choose another project in the sidebar."
            }
        }
        "mcp_connecting" => {
            if zh {
                "正在启动 MCP 服务…"
            } else {
                "Starting MCP servers…"
            }
        }
        "mcp_failed" => {
            if zh {
                "MCP 服务 {name} 没有启动：{error}"
            } else {
                "MCP server {name} did not start: {error}"
            }
        }
        "waiting_model" => {
            if zh {
                "正在等待 {name} 响应…"
            } else {
                "Waiting for {name}…"
            }
        }
        "request_failed" => {
            if zh {
                "请求「{name}」失败：{error}"
            } else {
                "The request to {name} failed: {error}"
            }
        }
        "cancelled" => {
            if zh {
                "已取消。"
            } else {
                "Cancelled."
            }
        }
        "denied" => {
            if zh {
                "用户拒绝了这次工具调用。"
            } else {
                "The user denied this tool call."
            }
        }
        "max_rounds" => {
            if zh {
                "已达到本轮工具调用上限，先停在这里。"
            } else {
                "Stopped after the tool-call limit for this turn."
            }
        }
        "empty" => {
            if zh {
                "模型没有返回内容。"
            } else {
                "The model returned an empty response."
            }
        }
        "new_chat" => {
            if zh {
                "新会话"
            } else {
                "New chat"
            }
        }
        "branch_title" => {
            if zh {
                "分支"
            } else {
                "Branch"
            }
        }
        "compacting" => {
            if zh {
                "上下文约 {used} / {window} tokens，已到 {percent}% 阈值，正在压缩较早的对话…"
            } else {
                "Context is about {used} of {window} tokens, past the {percent}% threshold. Compacting earlier messages…"
            }
        }
        "compacted" => {
            if zh {
                "已压缩较早的对话，继续回复。"
            } else {
                "Earlier messages were compacted. Continuing."
            }
        }
        "compact_failed" => {
            if zh {
                "压缩上下文失败（{error}），按原样继续发送。"
            } else {
                "Compaction failed ({error}); sending the full context."
            }
        }
        _ => key,
    };
    text.to_string()
}
