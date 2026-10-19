# Grok Desktop

这是 [grok-build](https://github.com/xai-org/grok-build) 的桌面分支（Apache-2.0，Copyright 2023-2026 SpaceXAI）。原来的命令行和全屏 TUI 仍然可以在这棵代码树里编译。本分支增加了原生桌面窗口，以及可插拔的第三方模型供应商。

上游基线：git `2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8`。单仓库快照记录在 [`SOURCE_REV`](SOURCE_REV)。上游原文 README 保留为 [`UPSTREAM_README.md`](UPSTREAM_README.md)。许可证：[`LICENSE`](LICENSE)。第三方声明：[`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES)。

English: [README.md](README.md).

## 这个分支增加了什么

- **Grok Desktop**（`grok-desktop`）：基于 Tauri 2 的窗口（Windows、macOS、Linux）。包含会话、文件和终端工具、扩展发现、快捷键，以及中文（zh-CN）和英文设置界面。
- **项目**：每段桌面对话都属于一个项目文件夹（代理的工作目录）。侧栏把这些文件夹列为仓库。可以用系统文件夹选择器添加项目、切换项目，并在当前项目里新建、重命名或删除对话。新对话使用当前选中的项目。会话保存在 `$GROK_HOME/desktop-sessions/<编码后的工作目录>/`，目录名编码与 CLI 的 `~/.grok/sessions/<编码后的工作目录>/` 相同。同一文件夹下的 CLI 会话会以只读方式在对话里打开。
- **分支**：在任意一条消息上选择「从这里分支」，会把第 1 到第 N 条消息复制到一段新对话（同一项目、同一供应商和模型），原对话保持不变。编辑过去的用户消息并重发，会从那条消息之前分出新分支，再用修改后的文本继续。如果分支点落在一轮工具调用的中间，已完成的工具结果会保留，没有输出的调用会被丢掉，模型收到的就是这段截断后的历史。
- **工作台**：窗口采用 Cursor 桌面的布局（新对话、搜索、自动化、自定义、仓库树），并吸收了 [grok-app](https://github.com/RongleCat/grok-app)（MIT，Copyright (c) 2026 RongleCat）的工作台想法：会话、对话看板、项目媒体预览、定时自动化。浅色和深色主题。窗口里不再嵌入经典终端；`xai-grok-pager` 仍是单独发布的二进制。
- **MCP 与图**：`config.toml` 里的 stdio MCP 会在桌面对话中被列出并调用。Mermaid 代码块用仓库里的 `mermaid-to-svg` 渲染。计划模式不会改文件，也不会执行命令。
- **供应商**（`grok-providers`）：xAI/Grok（Responses API，默认供应商）、OpenAI 兼容的 Chat Completions，以及 Anthropic Messages。支持流式输出和工具调用。内置 Grok、DeepSeek、月之暗面 Kimi、通用 OpenAI、Claude。这三种协议都可以添加自定义供应商。API 密钥优先放进系统钥匙串；钥匙串不可用时写入权限为 `0600` 的文件。密钥不会进入 git，也不会写进 `config.toml`。Grok 会复用 `grok login` 的登录会话。

## 结构

```
desktop/                  React + Vite 界面（zh-CN / en）
desktop/src-tauri/        Tauri 2 外壳（不嵌入终端）
crates/desktop/grok-desktop-core/   会话、工具、代理循环
crates/providers/grok-providers/    请求翻译、SSE、预设、密钥、CLI 合并
crates/codegen/xai-grok-shell/      上游代理；读取 providers.toml 和环境变量预设
crates/codegen/xai-grok-pager-bin/  上游 CLI（二进制名 xai-grok-pager）
```

桌面代理没有链接完整的 `xai-grok-shell`，也不会启动 `grok agent stdio`。如果像 grok-app 那样完全交给 CLI，就能用上钩子、沙箱、HTTP MCP 和语音，但会丢掉这套可分支的第三方会话和 `grok-providers` 的供应商循环。所以桌面对话保留这条循环，并加上 stdio MCP；钩子、语音、沙箱策略和 HTTP MCP 仍在独立 CLI 里。CLI 加载配置时在内存里合并供应商预设；你在配置里已经写过的 `[model.*]` 和 `[model_providers.*]` 不会被覆盖。

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

安装 [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 11 通常已自带）、Visual Studio 的“使用 C++ 的桌面开发”生成工具、rustup 提供的 Rust，以及 Node.js 22。在 Developer PowerShell 里同样执行 `cargo build -p grok-desktop`。

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

Linux 的 `deb` 和 `AppImage` 使用同一套工具链。先构建前端，再安装已发布的 Tauri CLI（2.10.1；没有 2.10.3）：

```sh
cargo install tauri-cli --version 2.10.1 --locked
cd desktop && npx tauri build
```

`desktop/src-tauri/Cargo.toml` 把 Tauri 相关 crate 锁在 2.10.3 这一组。CLI 只有 2.10.1。`@tauri-apps/api` 也固定在 2.10.1，主次版本必须和 Rust `tauri` crate 一致，否则 CLI 拒绝打包。`tauri-runtime`、`wry`、`tauri-macros` 的更新补丁不能混用，否则编不过。

## 编译并运行原来的 CLI / TUI

```sh
export PROTOC=/path/to/protoc   # 仅当 bin/protoc 无法通过 dotslash 运行时
cargo build -p xai-grok-pager-bin
./target/debug/xai-grok-pager
```

桌面窗口不再嵌入这个 TUI。需要经典 pager 时，运行上面的二进制，或发布资源里的 `grok-<target>`。

## 下载

推送名为 `v*` 的标签（例如 `v0.1.0`）会在 GitHub 托管的 runner 上运行 [`.github/workflows/release.yml`](.github/workflows/release.yml)。该工作流使用 [`tauri-apps/tauri-action`](https://github.com/tauri-apps/tauri-action)，并把安装包上传到 GitHub Release。在同一个工作流上选择 **Run workflow**（`workflow_dispatch`）会构建同样的矩阵，只上传 Actions 产物，不发布 Release。这些任务要等仓库放到 GitHub 上才会执行。

| 平台 | 桌面安装包 | CLI 文件 |
|------|------------|----------|
| Windows x64 | `.msi` 和 NSIS `.exe` | `grok-x86_64-pc-windows-msvc.exe` |
| macOS Apple Silicon | `.dmg` | `grok-aarch64-apple-darwin` |
| macOS Intel | `.dmg`（在 Apple Silicon runner 上交叉编译） | `grok-x86_64-apple-darwin` |
| Linux x64 | `.deb` 和 `.AppImage` | `grok-x86_64-unknown-linux-gnu` |

每个 `grok-<target>` 文件就是 `xai-grok-pager`。把它改名为 `grok` 或 `xai-grok-pager` 放进 `PATH`。CLI 构建需要 `protoc` 29.3，工作流会安装它。在配置下面的密钥之前，构建结果是未签名的。

分支推送和 pull request 仍然运行 [`.github/workflows/desktop.yml`](.github/workflows/desktop.yml)：测试供应商并编译桌面二进制，不制作安装包。

### 打开未签名的 macOS 应用

Gatekeeper 会在第一次打开未签名或未公证的应用时拦截。

1. 把 `Grok Desktop.app` 拖到 `/Applications`（也可以先不推出 `.dmg`）。
2. 按住 Control 点按应用，选择 **打开**，再在对话框里点 **打开**。
3. 如果系统只给出 **移到废纸篓**，打开 **系统设置 → 隐私与安全性**，找到被拦截的应用，选择 **仍要打开**。
4. 也可以在终端去掉隔离属性再打开：

```sh
xattr -dr com.apple.quarantine "/Applications/Grok Desktop.app"
open "/Applications/Grok Desktop.app"
```

未签名的 CLI 同样处理：`xattr -dr com.apple.quarantine ./grok-aarch64-apple-darwin`，然后 `chmod +x` 再执行。Intel 使用 `x86_64` 那个文件。

### 以后再加上代码签名和公证

发布工作流只在密钥非空时才把它传给构建。在仓库里加上这些 Secrets 即可打开签名，不必再改工作流。

**macOS**（Developer ID Application + 公证）。创建 Developer ID Application 证书，导出 `.p12`，再做 base64（`base64 -i certificate.p12 | tr -d '\n'`）。

| Secret | 内容 |
|--------|------|
| `APPLE_CERTIFICATE` | `.p12` 的 Base64 |
| `APPLE_CERTIFICATE_PASSWORD` | 该 `.p12` 的密码 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: 名称 (TEAMID)` |
| `APPLE_ID` | `notarytool` 使用的 Apple ID 邮箱 |
| `APPLE_PASSWORD` | 应用专用密码 |
| `APPLE_TEAM_ID` | 10 位 Team ID |

也可以改用 App Store Connect API 密钥，设置 `APPLE_API_ISSUER`、`APPLE_API_KEY`（密钥 id）和 `APPLE_API_KEY_PATH`（或密钥内容，与 Tauri 2 的 macOS 签名文档一致）。公证完成后，`.dmg` 可以直接打开，不必再走上面的步骤。

**Windows**（Authenticode）。导出代码签名 `.pfx` 并做 base64。

| Secret | 内容 |
|--------|------|
| `WINDOWS_CERTIFICATE` | `.pfx` 的 Base64 |
| `WINDOWS_CERTIFICATE_PASSWORD` | 该 `.pfx` 的密码 |

Windows 任务会导入证书，并在 `tauri build` 之前把指纹写进 `bundle.windows.certificateThumbprint`，摘要算法为 SHA-256，时间戳服务器为 `http://timestamp.digicert.com`。不设置这些密钥时，`.msi` 和 NSIS `.exe` 保持未签名。

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

### Grok / xAI

新桌面对话的默认供应商。基址 `https://api.x.ai/v1`。协议是 Responses API（`POST /v1/responses`），`reasoning.effort` 为 `high`。这里的 Grok 4.x 不走 Chat Completions。

默认模型不是写死的版本。

- 桌面应用每次启动时，如果设置里有密钥、存在 `grok login` 会话，或设置了 `XAI_API_KEY`，就请求 `GET https://api.x.ai/v1/models`。
- 在形如 `grok-<数字>` 的 id 里，按**十进制**版本号选最高的（`4.7` 高于 `4.20` 和 `4.6`。这与 xAI 的产品线一致：Grok 4.7 比 4.20 系列更新）。
- 后缀里含 `mini`、`vision`、`image`、`imagine`、`video`、`voice`、`audio`、`tts`、`stt`、`embed` 的 id 会跳过，除非返回的全是这类模型。同分时没有后缀的 id 优先（`grok-4.7` 先于 `grok-4.7-fast`）。
- 列表缓存在 `~/.grok/xai-model-cache.json`。桌面应用每次启动都刷新。对 CLI 来说，这份缓存 6 小时内视为新鲜。
- CLI 加载配置时不访问网络。有缓存就用缓存，否则用离线回退。`grok-providers refresh-models` 会拉取列表并写入 `providers.toml` 和 `config.toml`。
- 离线回退：`grok-4.7`，然后是上游 `default_models.json` 里的 `grok-4.6` 和 `grok-4.5`。后两个仍然可以手动选择。
- 在设置里取消 **始终使用最新的 Grok**，或设置 `XAI_MODEL`，即可固定模型。固定之后，之后的刷新不会覆盖它。

2026-10-19 核对过公开文档：[模型列表](https://docs.x.ai/docs/models) 和 [Grok 4.7](https://docs.x.ai/developers/grok-4-7)（模型 id `grok-4.7`，50 万上下文，Responses API，页面更新于 2026-10-10）。本快照里上游的 `crates/codegen/xai-grok-models/default_models.json` 仍把 `grok-4.6` 当作内置默认，并列出 `grok-4.5`。该文件没有 `grok-4.7`；上游测试里把它当作夹具 id 使用过。

这个预设的凭据顺序：设置里保存的密钥（钥匙串或 `0600` 文件），然后是 `grok login` 会话（`GROK_AUTH`、`$GROK_AUTH_PATH` 或 `$GROK_HOME/auth.json`），然后是 `XAI_API_KEY`，然后是 `GROK_CODE_XAI_API_KEY`，最后是 `auth.json` 里的 `xai::api_key`。同步到 CLI 的表使用 `api_backend = "responses"`，并且不写 `env_key`，所以上游自己的回退仍然让登录会话优先于 `XAI_API_KEY`。

```sh
export XAI_API_KEY=xai-...                 # 已经 grok login 成功时可以不设
export XAI_BASE_URL=https://api.x.ai/v1   # 可选
export XAI_MODEL=grok-4.5                 # 可选，固定模型；不设则跟随最新
cargo run -p grok-providers -- refresh-models
```

Responses API 返回的加密推理内容，下一轮桌面对话里还不会原样带回。

### DeepSeek

OpenAI 兼容。基址 `https://api.deepseek.com`（没有 `/v1`）。默认模型 `deepseek-chat`，也列出了 `deepseek-reasoner`。token 字段是 `max_tokens`。

```sh
export DEEPSEEK_API_KEY=sk-...
export DEEPSEEK_MODEL=deepseek-chat          # 可选
export DEEPSEEK_BASE_URL=https://api.deepseek.com   # 可选
```

在桌面应用里：设置 → 启用 DeepSeek → 设为当前供应商 → 粘贴密钥 → 保存。密钥进入钥匙串或 `0600` 文件。同一环境下单独运行 CLI 时，进程仍会读取 `DEEPSEEK_API_KEY`。

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

桌面聊天是一条新的代理循环，工具是上面那六个，再加上 stdio MCP 和 Mermaid 图。窗口里不再嵌入经典 TUI。语音、沙箱策略、ACP、hooks 和 HTTP MCP 仍在 `xai-grok-pager` 里。桌面循环一次只跑一轮对话；除非在设置里改成自动，否则写文件和执行命令前会先询问。模型名的默认值只是建议，账号所用的 id 不同时请改掉。定时自动化只在窗口开着的时候触发，退出之后不会在后台跑。
