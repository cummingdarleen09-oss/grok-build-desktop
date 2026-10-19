# Grok Desktop

这是 [grok-build](https://github.com/xai-org/grok-build) 的桌面分支（Apache-2.0，Copyright 2023-2026 SpaceXAI）。原来的命令行和全屏 TUI 仍然可以在这棵代码树里编译。本分支增加了原生桌面窗口，以及可插拔的第三方模型供应商。

上游基线：git `2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8`。单仓库快照记录在 [`SOURCE_REV`](SOURCE_REV)。上游原文 README 保留为 [`UPSTREAM_README.md`](UPSTREAM_README.md)。许可证：[`LICENSE`](LICENSE)。第三方声明：[`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES)。

English: [README.md](README.md).

## 这个分支增加了什么

- **Grok Desktop**（`grok-desktop`）：基于 Tauri 2 的窗口（Windows、macOS、Linux）。包含会话、文件和终端工具、扩展发现、快捷键，以及中文（zh-CN）和英文设置界面。
- **窗口里的经典 TUI**：终端页用 PTY 启动原来的 `xai-grok-pager`。斜杠命令、插件、hooks、MCP、沙箱，以及上游 TUI 的其余能力都还在。
- **供应商**（`grok-providers`）：OpenAI 兼容的 Chat Completions，以及 Anthropic Messages。支持流式输出和工具调用。内置 DeepSeek、月之暗面 Kimi、通用 OpenAI、Claude。两种协议都可以添加自定义供应商。API 密钥优先放进系统钥匙串；钥匙串不可用时写入权限为 `0600` 的文件。密钥不会进入 git，也不会写进 `config.toml`。

## 结构

```
desktop/                  React + Vite 界面（zh-CN / en）
desktop/src-tauri/        Tauri 2 外壳，以及经典 TUI 的 PTY
crates/desktop/grok-desktop-core/   会话、工具、代理循环
crates/providers/grok-providers/    请求翻译、SSE、预设、密钥、CLI 合并
crates/codegen/xai-grok-shell/      上游代理；读取 providers.toml 和环境变量预设
crates/codegen/xai-grok-pager-bin/  上游 CLI（二进制名 xai-grok-pager）
```

桌面代理没有链接完整的 `xai-grok-shell`。模型请求走 `grok-providers`，工具只有 `read_file`、`write_file`、`search_replace`、`grep`、`list_dir`、`run_terminal`。要用上游的全部能力，请运行原来的二进制。CLI 加载配置时在内存里合并供应商预设；你在配置里已经写过的 `[model.*]` 和 `[model_providers.*]` 不会被覆盖。

## 环境

- Rust，版本由 [`rust-toolchain.toml`](rust-toolchain.toml) 固定（1.94）。
- Node.js 22 和 npm，用来构建桌面界面。
- 原来的 CLI 需要 [protoc](https://github.com/protocolbuffers/protobuf/releases) 29.3。上游在 [`bin/protoc`](bin/protoc) 放了 [DotSlash](https://dotslash-cli.com) 包装。如果没有安装 `dotslash`，把 `PROTOC` 指到一个 `protoc` 可执行文件。

### Linux 桌面依赖

Ubuntu / Debian：

```sh
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev \
  librsvg2-dev patchelf libssl-dev build-essential pkg-config \
  libayatana-appindicator3-dev file curl
```

链接阶段需要 `libxdo`（`-lxdo`）。请安装 `libxdo-dev`，不能只装运行时库。

### macOS

安装 Xcode 命令行工具、Rust 和 Node.js 22。系统自带的 WebKit 就是网页视图，不需要 GTK 和 libxdo。

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Node 22 从 <https://nodejs.org> 安装。

### Windows

安装 [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 11 通常已自带）、Visual Studio 的“使用 C++ 的桌面开发”生成工具、rustup 提供的 Rust，以及 Node.js 22。在 Developer PowerShell 里同样执行 `cargo build -p grok-desktop`。经典 TUI 页通过 `portable-pty` 使用 ConPTY。

## 编译并运行桌面应用

```sh
cd desktop
npm ci
npm run build          # 生成 desktop/dist，Tauri 二进制会把它嵌进去
cd ..
cargo build -p grok-desktop
./target/debug/grok-desktop
```

只看界面时可以开开发服务器。不在 Tauri 窗口里时，它使用内存里的示例数据：

```sh
cd desktop
npm run dev            # http://127.0.0.1:43123
```

发布构建：

```sh
cd desktop && npm run build && cd ..
cargo build -p grok-desktop --release
./target/release/grok-desktop
```

Linux 的 `deb` 和 `AppImage` 使用同一套工具链。先构建前端，再在仓库里安装与源码锁定一致的 Tauri CLI：

```sh
cargo install tauri-cli --version 2.10.3 --locked
cd desktop && npx tauri build
```

`desktop/src-tauri/Cargo.toml` 把 Tauri 相关 crate 锁在 2.10.3 这一组。`tauri-runtime`、`wry`、`tauri-macros` 的更新补丁不能混用，否则编不过。

## 编译并运行原来的 CLI / TUI

```sh
export PROTOC=/path/to/protoc   # 仅当 bin/protoc 无法通过 dotslash 运行时
cargo build -p xai-grok-pager-bin
./target/debug/xai-grok-pager
```

桌面里的终端页按这个顺序找二进制：`GROK_BIN`，然后是 `PATH` 上的 `grok` 或 `xai-grok-pager`，再从当前目录向上找 `target/debug/xai-grok-pager` 和 `target/release/xai-grok-pager`。

## 配置供应商

文件在 `$GROK_HOME` 或 `~/.grok`：

| 文件 | 作用 |
|------|------|
| `providers.toml` | 供应商列表。不要写 API 密钥。 |
| `desktop.toml` | 语言、工作目录、权限模式、当前供应商。 |
| `config.toml` | 上游 CLI 配置。`grok-providers sync` 把已启用的供应商写进去。 |
| `provider-secrets.json` | 钥匙串拒绝写入时的备用文件，权限 `0600`。 |
| `desktop-sessions/` | 桌面聊天记录。 |

钥匙串服务名是 `grok-desktop`。设置 `GROK_DISABLE_PROVIDER_PRESETS=1` 可以关掉 CLI 的自动预设。

打印示例文件：

```sh
cargo run -p grok-providers -- presets > "$HOME/.grok/providers.toml"
cargo run -p grok-providers -- sync
```

### DeepSeek

OpenAI 兼容。基址 `https://api.deepseek.com`（没有 `/v1`）。默认模型 `deepseek-chat`，也列出了 `deepseek-reasoner`。token 字段是 `max_tokens`。

```sh
export DEEPSEEK_API_KEY=sk-...
export DEEPSEEK_MODEL=deepseek-chat          # 可选
export DEEPSEEK_BASE_URL=https://api.deepseek.com   # 可选
```

在桌面应用里：设置 → 启用 DeepSeek → 设为当前供应商 → 粘贴密钥 → 保存。密钥进入钥匙串或 `0600` 文件。打开经典 TUI 时，进程会拿到环境变量 `DEEPSEEK_API_KEY`。

### 月之暗面 Kimi

OpenAI 兼容。基址 `https://api.moonshot.cn/v1`。默认模型 `moonshot-v1-128k`。

```sh
export MOONSHOT_API_KEY=sk-...
# 也可以用 KIMI_API_KEY
export MOONSHOT_MODEL=moonshot-v1-128k       # 可选
export MOONSHOT_BASE_URL=https://api.moonshot.cn/v1   # 可选
```

### 通用 OpenAI 兼容接口

基址、密钥和模型都可以改。预设默认值是 `https://api.openai.com/v1`、`gpt-4o`，token 字段为 `max_completion_tokens`（较新的 OpenAI 模型会拒绝 `max_tokens`）。DeepSeek 这类服务请用 DeepSeek 预设，它发送的是 `max_tokens`。

```sh
export OPENAI_API_KEY=sk-...
export OPENAI_BASE_URL=https://api.openai.com/v1
export OPENAI_MODEL=gpt-4o
```

其他 OpenAI 兼容服务（例如本机代理）写成自定义供应商：

```toml
[[providers]]
id = "lab-proxy"
name = "实验室代理"
format = "openai"
base_url = "http://127.0.0.1:4000/v1"
model = "local-model"
env_key = "LAB_PROXY_API_KEY"
context_window = 32000
auth = "bearer"
enabled = true
max_tokens = 4096
token_field = "max_tokens"
builtin = false
```

流式响应用 `POST {base_url}/chat/completions` 的 Server-Sent Events。工具调用使用 Chat Completions 的 `tools` / `tool_calls`。

### Anthropic / Claude Messages

基址 `https://api.anthropic.com/v1`，模型 `claude-sonnet-4-5`。认证头是 `x-api-key`（不是 `Authorization: Bearer`），另外带 `anthropic-version: 2023-06-01`。流式协议是 Messages 的 SSE（`content_block_delta`、`tool_use`）。

```sh
export ANTHROPIC_API_KEY=sk-ant-...
export ANTHROPIC_BASE_URL=https://api.anthropic.com/v1   # 可选
export ANTHROPIC_MODEL=claude-sonnet-4-5                 # 可选
```

Claude 格式的代理在 `providers.toml` 里写 `format = "anthropic"` 和 `auth = "x_api_key"`，或者在设置页用自定义供应商。

也可以手写上游的配置表。参见 [`UPSTREAM_README.md`](UPSTREAM_README.md) 和 `crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md`。`providers.toml` 里启用的供应商只有在该 id 尚未出现时才会合并进去。

## 测试

```sh
cargo test -p grok-providers -p grok-desktop-core
cargo build -p grok-desktop
cargo build -p xai-grok-pager-bin
```

`grok-providers` 的测试覆盖两种协议的请求翻译和 SSE 解析，并用模拟 HTTP 服务检查流式工具调用、Anthropic 的 `x-api-key`，以及错误响应里的密钥脱敏。

## CI

[`.github/workflows/desktop.yml`](.github/workflows/desktop.yml) 在 Linux 上构建桌面应用并运行新增测试，在 macOS 和 Windows 上构建桌面应用。完整的上游 CLI 体积很大，而且需要 `protoc`，请在本机用上面的命令编译。

## 还没有做成第二份 TUI 的部分

桌面聊天是一条新的代理循环，工具就是上面那六个。它没有重做上游的 mermaid、语音、沙箱策略、ACP、hooks，也没有完整的 MCP 客户端。这些请打开终端页，或直接运行 `xai-grok-pager`。桌面循环一次只跑一轮对话；除非在设置里改成自动，否则写文件和执行命令前会先询问。模型名的默认值只是建议，账号所用的 id 不同时请改掉。
