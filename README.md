# Grok Desktop

A desktop fork of [grok-build](https://github.com/xai-org/grok-build) (Apache-2.0, Copyright 2023-2026 SpaceXAI). The original CLI and fullscreen TUI still build from this tree. This fork adds a native desktop window and a pluggable layer for third-party model providers.

Upstream baseline: git `2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8`. The monorepo snapshot is recorded in [`SOURCE_REV`](SOURCE_REV). The original project README is preserved as [`UPSTREAM_README.md`](UPSTREAM_README.md). License: [`LICENSE`](LICENSE). Third-party notices: [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES).

中文说明：[README.zh-CN.md](README.zh-CN.md).

## What this fork adds

- **Grok Desktop** (`grok-desktop`): a Tauri 2 window (Windows, macOS, Linux) with chat sessions, file and shell tools, extension discovery, keyboard shortcuts, and settings in English and Chinese (zh-CN).
- **Classic TUI inside the window**: the Terminal page starts the original `xai-grok-pager` binary on a PTY, so slash commands, plugins, hooks, MCP, sandboxing, and the rest of the upstream TUI stay available.
- **Providers** (`grok-providers`): OpenAI-compatible Chat Completions and Anthropic Messages, with streaming and tool calls. Built-in presets for DeepSeek, Moonshot Kimi, generic OpenAI, and Claude. Custom providers of either format. API keys go to the OS keychain when it is available, otherwise to a `0600` file. They are never written into git or into `config.toml`.

## Architecture

```
desktop/                  React + Vite UI (zh-CN / en)
desktop/src-tauri/        Tauri 2 shell, PTY host for the classic TUI
crates/desktop/grok-desktop-core/   sessions, tools, agent loop
crates/providers/grok-providers/    translation, SSE, presets, secrets, CLI merge
crates/codegen/xai-grok-shell/      upstream agent; loads providers.toml + env presets
crates/codegen/xai-grok-pager-bin/  upstream CLI binary (xai-grok-pager)
```

The desktop agent does not link the full `xai-grok-shell` runtime. It calls `grok-providers` for model traffic and a small tool set (`read_file`, `write_file`, `search_replace`, `grep`, `list_dir`, `run_terminal`). The original binary is still the way to use every upstream capability. Provider presets are merged in memory when the CLI loads config, so existing `[model.*]` and `[model_providers.*]` entries in your config win.

## Requirements

- Rust, pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (1.94).
- Node.js 22 and npm, for the desktop UI.
- For the original CLI: [protoc](https://github.com/protocolbuffers/protobuf/releases) 29.3. Upstream ships a [DotSlash](https://dotslash-cli.com) wrapper at [`bin/protoc`](bin/protoc). If `dotslash` is not installed, set `PROTOC` to a `protoc` binary instead.

### Linux packages for the desktop app

Ubuntu / Debian:

```sh
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev \
  librsvg2-dev patchelf libssl-dev build-essential pkg-config \
  libayatana-appindicator3-dev file curl
```

`libxdo` is required at link time (`-lxdo`). Install `libxdo-dev`, not only the runtime library.

### macOS

Install the Xcode command line tools, Rust, and Node.js 22. System WebKit is the webview; GTK and libxdo are not used.

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# install Node 22 from https://nodejs.org
```

### Windows

Install [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) (already present on Windows 11), the Visual Studio C++ build tools (Desktop development with C++), Rust via rustup, and Node.js 22. Then use the same `cargo build -p grok-desktop` command from a Developer PowerShell. The classic TUI page uses a ConPTY host through `portable-pty`.

## Build and run the desktop app

```sh
cd desktop
npm ci
npm run build          # writes desktop/dist, which the Tauri binary embeds
cd ..
cargo build -p grok-desktop
./target/debug/grok-desktop
```

The UI dev server (mock data when it is not inside the Tauri window) is:

```sh
cd desktop
npm run dev            # http://127.0.0.1:43123
```

Release binary:

```sh
cd desktop && npm run build && cd ..
cargo build -p grok-desktop --release
./target/release/grok-desktop
```

Linux bundles (`deb`, `AppImage`) use the same toolchain. From `desktop/src-tauri`, after the frontend build:

```sh
cargo install tauri-cli --version 2.10.3 --locked
cd desktop && npx tauri build
```

Pin the Tauri crates to the 2.10.3 set already listed in `desktop/src-tauri/Cargo.toml`. Newer patch releases of `tauri-runtime`, `wry`, and `tauri-macros` do not compile together.

## Build and run the original CLI / TUI

```sh
export PROTOC=/path/to/protoc   # only if `bin/protoc` cannot run via dotslash
cargo build -p xai-grok-pager-bin
./target/debug/xai-grok-pager
```

The desktop Terminal page looks for `GROK_BIN`, then `grok` / `xai-grok-pager` on `PATH`, then `target/debug/xai-grok-pager` and `target/release/xai-grok-pager` walking up from the current directory.

## Configure providers

Files live under `$GROK_HOME` or `~/.grok`:

| File | Purpose |
|------|---------|
| `providers.toml` | Provider list. No API keys. |
| `desktop.toml` | Language, working directory, permission mode, active provider. |
| `config.toml` | Upstream CLI config. `grok-providers sync` upserts enabled providers here. |
| `provider-secrets.json` | Fallback key file, mode `0600`, used when the OS keychain refuses the write. |
| `desktop-sessions/` | Desktop chat transcripts. |

The keychain service name is `grok-desktop`. Set `GROK_DISABLE_PROVIDER_PRESETS=1` to stop the CLI from auto-inserting presets.

Print an example file:

```sh
cargo run -p grok-providers -- presets > "$HOME/.grok/providers.toml"
cargo run -p grok-providers -- sync
```

### DeepSeek

OpenAI-compatible. Base URL `https://api.deepseek.com` (no `/v1`). Default model `deepseek-chat`. `deepseek-reasoner` is also listed. Token field `max_tokens`.

```sh
export DEEPSEEK_API_KEY=sk-...
export DEEPSEEK_MODEL=deepseek-chat          # optional
export DEEPSEEK_BASE_URL=https://api.deepseek.com   # optional
```

In the desktop app: Settings → enable DeepSeek → mark it active → paste the key → Save. The key is stored in the keychain or the `0600` file, and the classic TUI process receives it as `DEEPSEEK_API_KEY`.

### Moonshot Kimi

OpenAI-compatible. Base URL `https://api.moonshot.cn/v1`. Default model `moonshot-v1-128k`.

```sh
export MOONSHOT_API_KEY=sk-...
# KIMI_API_KEY is accepted as a fallback
export MOONSHOT_MODEL=moonshot-v1-128k       # optional
export MOONSHOT_BASE_URL=https://api.moonshot.cn/v1   # optional
```

### Generic OpenAI-compatible API

Base URL, API key, and model are all configurable. Preset defaults: `https://api.openai.com/v1`, `gpt-4o`, token field `max_completion_tokens` (newer OpenAI models reject `max_tokens`). DeepSeek-style servers should stay on the DeepSeek preset, which sends `max_tokens`.

```sh
export OPENAI_API_KEY=sk-...
export OPENAI_BASE_URL=https://api.openai.com/v1
export OPENAI_MODEL=gpt-4o
```

Any other OpenAI-compatible server (a local proxy, for example) is a custom provider:

```toml
[[providers]]
id = "lab-proxy"
name = "Lab proxy"
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

Streaming uses Server-Sent Events from `POST {base_url}/chat/completions`. Tool calls use the Chat Completions `tools` / `tool_calls` shape.

### Anthropic / Claude Messages API

Base URL `https://api.anthropic.com/v1`, model `claude-sonnet-4-5`, auth header `x-api-key` (not `Authorization: Bearer`), plus `anthropic-version: 2023-06-01`. Streaming is the Messages SSE stream (`content_block_delta`, `tool_use`).

```sh
export ANTHROPIC_API_KEY=sk-ant-...
export ANTHROPIC_BASE_URL=https://api.anthropic.com/v1   # optional
export ANTHROPIC_MODEL=claude-sonnet-4-5                 # optional
```

A Claude-format proxy uses `format = "anthropic"` and `auth = "x_api_key"` in `providers.toml`, or the custom-provider form in Settings.

You can also write the upstream tables by hand. See [`UPSTREAM_README.md`](UPSTREAM_README.md) and `crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md`. Enabled providers from `providers.toml` are merged only when that id is not already present.

## Tests

```sh
cargo test -p grok-providers -p grok-desktop-core
cargo build -p grok-desktop
cargo build -p xai-grok-pager-bin
```

`grok-providers` tests cover request translation and SSE parsing for both formats, including a mocked HTTP server (streaming tool calls, Anthropic `x-api-key`, and redacted HTTP errors).

## CI

[`.github/workflows/desktop.yml`](.github/workflows/desktop.yml) builds the desktop app and runs the new tests on Linux, and builds the desktop app on macOS and Windows. The full upstream CLI is built locally with the command above; it is large and needs `protoc`.

## What is not a second copy of the TUI

The desktop chat is a new agent loop with the six tools listed above. It does not reimplement upstream mermaid rendering, voice, the sandbox policy engine, ACP, hooks, or the full MCP client. Open the Terminal page (or run `xai-grok-pager`) for those. The desktop loop runs one turn at a time and asks before writes and shell commands unless Settings is set to auto. Model name defaults are suggestions; change them if your account uses a different id.
