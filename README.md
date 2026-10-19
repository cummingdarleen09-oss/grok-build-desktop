# Grok Desktop

A desktop fork of [grok-build](https://github.com/xai-org/grok-build) (Apache-2.0, Copyright 2023-2026 SpaceXAI). The original CLI and fullscreen TUI still build from this tree. This fork adds a native desktop window and a pluggable layer for third-party model providers.

Upstream baseline: git `2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8`. The monorepo snapshot is recorded in [`SOURCE_REV`](SOURCE_REV). The original project README is preserved as [`UPSTREAM_README.md`](UPSTREAM_README.md). License: [`LICENSE`](LICENSE). Third-party notices: [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES).

中文说明：[README.zh-CN.md](README.zh-CN.md).

## What this fork adds

- **Grok Desktop** (`grok-desktop`): a Tauri 2 window (Windows, macOS, Linux) with chat sessions, file and shell tools, extension discovery, keyboard shortcuts, and settings in English and Chinese (zh-CN).
- **Projects**: every desktop conversation belongs to a project folder (the working directory the agent uses). The sidebar groups chats by that folder. Add a folder with the native picker, switch projects, and create, rename, or delete conversations inside the selected one. New chats use the selected project. Sessions are stored at `$GROK_HOME/desktop-sessions/<encoded-cwd>/`, the same directory encoding the classic TUI uses under `~/.grok/sessions/<encoded-cwd>/`. Classic TUI sessions for that folder show up in the same group.
- **Branches**: "Branch from here" / "从这里分支" copies messages `1..N` into a new conversation (same project, provider, and model) and leaves the original untouched. Editing a past user message and resending starts a branch that continues from the edited text. A branch point in the middle of a tool-call round keeps finished tool results and drops calls that never received output, so the model sees exactly that prefix.
- **Classic TUI inside the window**: the Terminal page starts the original `xai-grok-pager` binary on a PTY, so slash commands, plugins, hooks, MCP, sandboxing, and the rest of the upstream TUI stay available.
- **Providers** (`grok-providers`): xAI/Grok (Responses API, the default), OpenAI-compatible Chat Completions, and Anthropic Messages, with streaming and tool calls. Built-in presets for Grok, DeepSeek, Moonshot Kimi, generic OpenAI, and Claude. Custom providers of any of those formats. API keys go to the OS keychain when it is available, otherwise to a `0600` file. They are never written into git or into `config.toml`. A `grok login` session is reused for Grok.

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
cargo install tauri-cli --version 2.10.1 --locked
cd desktop && npx tauri build
```

Pin the Tauri crates to the 2.10.3 set already listed in `desktop/src-tauri/Cargo.toml`. The published `tauri-cli` / `@tauri-apps/cli` line stops at 2.10.1. Newer patch releases of `tauri-runtime`, `wry`, and `tauri-macros` do not compile together.

## Build and run the original CLI / TUI

```sh
export PROTOC=/path/to/protoc   # only if `bin/protoc` cannot run via dotslash
cargo build -p xai-grok-pager-bin
./target/debug/xai-grok-pager
```

The desktop Terminal page looks for `GROK_BIN`, then `grok` / `xai-grok-pager` on `PATH`, then `target/debug/xai-grok-pager` and `target/release/xai-grok-pager` walking up from the current directory.

## Downloads

Pushing a tag named `v*` (for example `v0.1.0`) runs [`.github/workflows/release.yml`](.github/workflows/release.yml) on GitHub-hosted runners. The workflow uses [`tauri-apps/tauri-action`](https://github.com/tauri-apps/tauri-action) and uploads a GitHub Release. **Run workflow** on that same file (`workflow_dispatch`) builds the same matrix and uploads Actions artifacts without publishing a Release. These jobs run after this repository is on GitHub.

| Platform | Desktop assets | CLI asset |
|----------|----------------|-----------|
| Windows x64 | `.msi` and NSIS `.exe` | `grok-x86_64-pc-windows-msvc.exe` |
| macOS Apple Silicon | `.dmg` | `grok-aarch64-apple-darwin` |
| macOS Intel | `.dmg` (cross-built on the Apple Silicon runner) | `grok-x86_64-apple-darwin` |
| Linux x64 | `.deb` and `.AppImage` | `grok-x86_64-unknown-linux-gnu` |

Each `grok-<target>` file is the `xai-grok-pager` binary. Rename it to `grok` or `xai-grok-pager`, put it on `PATH`, or set `GROK_BIN`. The desktop app also looks next to its own executable for those names. The CLI build needs `protoc` 29.3; the workflow installs it. Builds are unsigned until the secrets below are set.

Branch pushes and pull requests still run [`.github/workflows/desktop.yml`](.github/workflows/desktop.yml), which tests providers and compiles the desktop binary without producing installers.

### Opening an unsigned macOS app

Gatekeeper blocks an unsigned or unnotarized app the first time it is opened.

1. Move `Grok Desktop.app` to `/Applications` (or leave the `.dmg` mounted).
2. Control-click the app and choose **Open**, then **Open** again in the dialog.
3. If macOS only offers **Move to Trash**, open **System Settings → Privacy & Security**, scroll to the blocked app, and choose **Open Anyway**.
4. From Terminal, the quarantine flag can be cleared and the app opened:

```sh
xattr -dr com.apple.quarantine "/Applications/Grok Desktop.app"
open "/Applications/Grok Desktop.app"
```

An unsigned CLI binary can be run the same way: `xattr -dr com.apple.quarantine ./grok-aarch64-apple-darwin`, then `chmod +x` and execute it. Intel builds use the `x86_64` asset.

### Code signing and notarization later

The release workflow forwards a secret only when it is non-empty, so adding these repository secrets turns signing on without another code change.

**macOS** (Developer ID Application + notarization). Create a Developer ID Application certificate, export the `.p12`, and base64-encode it (`base64 -i certificate.p12 | tr -d '\n'`).

| Secret | Value |
|--------|--------|
| `APPLE_CERTIFICATE` | Base64 of the `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password for that `.p12` |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple ID email used by `notarytool` |
| `APPLE_PASSWORD` | App-specific password |
| `APPLE_TEAM_ID` | 10-character Team ID |

An App Store Connect API key can replace the Apple ID password. Set `APPLE_API_ISSUER`, `APPLE_API_KEY` (the key id), and `APPLE_API_KEY_PATH` (or the key contents, matching the Tauri 2 macOS signing guide). After notarization, the `.dmg` opens without the steps above.

**Windows** (Authenticode). Export a code-signing `.pfx` and base64-encode it.

| Secret | Value |
|--------|--------|
| `WINDOWS_CERTIFICATE` | Base64 of the `.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password for that `.pfx` |

The Windows job imports the certificate and writes its thumbprint into `bundle.windows.certificateThumbprint` before `tauri build`, with SHA-256 and `http://timestamp.digicert.com`. Leave the secrets unset to keep the `.msi` and NSIS `.exe` unsigned.

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

### Grok / xAI

Default provider for new desktop chats. Base URL `https://api.x.ai/v1`. The wire protocol is the Responses API (`POST /v1/responses`) with `reasoning.effort` set to `high`. Grok 4.x does not use Chat Completions here.

The default model is not a hard-coded version.

- On desktop launch, if a settings key, a `grok login` session, or `XAI_API_KEY` is available, the app calls `GET https://api.x.ai/v1/models`.
- It picks the highest **decimal** version among ids shaped like `grok-<number>` (`4.7` outranks `4.20` and `4.6`, which matches xAI's lineup: Grok 4.7 is newer than the 4.20 series).
- Suffix tokens `mini`, `vision`, `image`, `imagine`, `video`, `voice`, `audio`, `tts`, `stt`, and `embed` are skipped unless every returned id is one of those. An unsuffixed id wins a tie (`grok-4.7` before `grok-4.7-fast`).
- The list is cached at `~/.grok/xai-model-cache.json`. The desktop app refreshes it on every launch. The cache is treated as fresh for 6 hours for the CLI.
- The CLI does not call the network while loading config. It uses the cache when one exists, otherwise the offline fallback. `grok-providers refresh-models` fetches the list and writes both `providers.toml` and `config.toml`.
- Offline fallback: `grok-4.7`, then the ids in upstream `default_models.json` (`grok-4.6`, `grok-4.5`), which stay selectable.
- Uncheck **Always use the newest Grok** in Settings, or set `XAI_MODEL`, to pin a model. A pin is not overwritten by a later refresh.

`grok-4.7` was confirmed on 2026-10-19 from the public docs: [models](https://docs.x.ai/docs/models) and [Grok 4.7](https://docs.x.ai/developers/grok-4-7) (model id `grok-4.7`, 500k context, Responses API, page updated 2026-10-10). Upstream `crates/codegen/xai-grok-models/default_models.json` in this snapshot still bakes `grok-4.6` as its default and also lists `grok-4.5`. `grok-4.7` is not in that file; it does appear as a fixture id in upstream tests.

Credential order for this preset: a key saved in Settings (keychain or the `0600` file), then the `grok login` session (`GROK_AUTH`, `$GROK_AUTH_PATH`, or `$GROK_HOME/auth.json`), then `XAI_API_KEY`, then `GROK_CODE_XAI_API_KEY`, then the `xai::api_key` entry in `auth.json`. The synced CLI table uses `api_backend = "responses"` and does not set `env_key`, so upstream's own fallback still prefers a login session over `XAI_API_KEY`.

```sh
export XAI_API_KEY=xai-...                 # optional when `grok login` already succeeded
export XAI_BASE_URL=https://api.x.ai/v1   # optional
export XAI_MODEL=grok-4.5                 # optional pin; omit to follow latest
cargo run -p grok-providers -- refresh-models
```

Encrypted reasoning content that the Responses API returns is not yet sent back on the next desktop turn.

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
