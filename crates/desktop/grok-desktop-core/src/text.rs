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
        "no_provider" => {
            if zh {
                "还没有选择模型供应商。请先在设置中启用 DeepSeek、Kimi、OpenAI 或 Claude。"
            } else {
                "No model provider is selected. Enable DeepSeek, Kimi, OpenAI, or Claude in Settings."
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
        _ => key,
    };
    text.to_string()
}
