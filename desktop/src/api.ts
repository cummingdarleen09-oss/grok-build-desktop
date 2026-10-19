import type { Lang } from "./i18n";

export type ProviderFormat = "openai" | "anthropic";

export interface Provider {
  id: string;
  name: string;
  format: ProviderFormat;
  base_url: string;
  model: string;
  env_key: string;
  env_key_fallbacks: string[];
  context_window: number;
  extra_headers: Record<string, string>;
  env_http_headers: Record<string, string>;
  auth: "bearer" | "x_api_key";
  enabled: boolean;
  temperature: number | null;
  max_tokens: number;
  token_field: "max_tokens" | "max_completion_tokens";
  builtin: boolean;
  model_hints: string[];
}

export interface ProviderStore {
  active: string | null;
  providers: Provider[];
}

export interface Settings {
  lang: Lang;
  permission: "ask" | "auto";
  cwd: string;
  active_provider: string | null;
  disabled_tools: string[];
}

export interface StoredTool {
  id: string;
  name: string;
  arguments: string;
  output?: string | null;
  is_error: boolean;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  reasoning: string;
  tool_calls: StoredTool[];
}

export interface Session {
  id: string;
  title: string;
  cwd: string;
  provider_id: string;
  model: string;
  created_at: string;
  updated_at: string;
  messages: ChatMessage[];
}

export interface SessionSummary {
  id: string;
  title: string;
  model: string;
  provider_id: string;
  updated_at: string;
}

export interface ExtensionRecord {
  kind: string;
  name: string;
  path: string;
  detail: string;
}

export interface SecretStatus {
  id: string;
  location: "keyring" | "file" | null;
  from_env: boolean;
}

export interface Bootstrap {
  settings: Settings;
  providers: ProviderStore;
  sessions: SessionSummary[];
  extensions: ExtensionRecord[];
  secrets: SecretStatus[];
  grok_binary: string | null;
  grok_home: string;
}

export type AgentEvent =
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "approval_needed"; id: string; name: string; arguments: string }
  | { type: "tool_started"; id: string; name: string; arguments: string }
  | { type: "tool_finished"; id: string; output: string; is_error: boolean }
  | { type: "error"; message: string }
  | { type: "done" };

export interface AgentEnvelope {
  session_id: string;
  event: AgentEvent;
}

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const memorySessions = new Map<string, Session>();
const memory = seed();

function seed(): Bootstrap {
  const providers: Provider[] = [
    preset("deepseek", "DeepSeek", "openai", "https://api.deepseek.com", "deepseek-chat", "DEEPSEEK_API_KEY", "bearer", "max_tokens", 128000, ["deepseek-chat", "deepseek-reasoner"], true),
    preset("kimi", "Moonshot Kimi", "openai", "https://api.moonshot.cn/v1", "moonshot-v1-128k", "MOONSHOT_API_KEY", "bearer", "max_tokens", 128000, ["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k", "kimi-latest"], false),
    preset("openai", "OpenAI-compatible", "openai", "https://api.openai.com/v1", "gpt-4o", "OPENAI_API_KEY", "bearer", "max_completion_tokens", 128000, ["gpt-4o", "gpt-4.1"], false),
    preset("anthropic", "Anthropic Claude", "anthropic", "https://api.anthropic.com/v1", "claude-sonnet-4-5", "ANTHROPIC_API_KEY", "x_api_key", "max_tokens", 200000, ["claude-sonnet-4-5", "claude-opus-4-5"], false),
  ];
  const session: Session = {
    id: "demo",
    title: "Read the provider layer",
    cwd: "/workspace",
    provider_id: "deepseek",
    model: "deepseek-chat",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    messages: [
      {
        id: "u1",
        role: "user",
        text: "Where do DeepSeek requests get their URL?",
        reasoning: "",
        tool_calls: [],
      },
      {
        id: "a1",
        role: "assistant",
        text: "DeepSeek uses the OpenAI-compatible root https://api.deepseek.com, and the client appends chat/completions.",
        reasoning: "Check the preset table, then the endpoint joiner.",
        tool_calls: [
          {
            id: "t1",
            name: "grep",
            arguments: '{"pattern":"api.deepseek.com"}',
            output: "crates/providers/grok-providers/src/presets.rs: base_url = https://api.deepseek.com",
            is_error: false,
          },
        ],
      },
    ],
  };
  memorySessions.set(session.id, session);
  return {
    settings: {
      lang: "zh-CN",
      permission: "ask",
      cwd: "/workspace",
      active_provider: "deepseek",
      disabled_tools: [],
    },
    providers: { active: "deepseek", providers },
    sessions: [summary(session)],
    extensions: [
      {
        kind: "tool",
        name: "read_file",
        path: "desktop",
        detail: "built in",
      },
    ],
    secrets: providers.map((provider) => ({
      id: provider.id,
      location: provider.id === "deepseek" ? "file" : null,
      from_env: false,
    })),
    grok_binary: null,
    grok_home: "~/.grok",
  };
}

function preset(
  id: string,
  name: string,
  format: ProviderFormat,
  base_url: string,
  model: string,
  env_key: string,
  auth: Provider["auth"],
  token_field: Provider["token_field"],
  context_window: number,
  model_hints: string[],
  enabled: boolean,
): Provider {
  return {
    id,
    name,
    format,
    base_url,
    model,
    env_key,
    env_key_fallbacks: id === "kimi" ? ["KIMI_API_KEY"] : [],
    context_window,
    extra_headers: format === "anthropic" ? { "anthropic-version": "2023-06-01" } : {},
    env_http_headers: format === "anthropic" ? { "x-api-key": "ANTHROPIC_API_KEY" } : {},
    auth,
    enabled,
    temperature: null,
    max_tokens: 8192,
    token_field,
    builtin: true,
    model_hints,
  };
}

function summary(session: Session): SessionSummary {
  return {
    id: session.id,
    title: session.title,
    model: session.model,
    provider_id: session.provider_id,
    updated_at: session.updated_at,
  };
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: call } = await import("@tauri-apps/api/core");
  return call<T>(command, args);
}

export async function loadBootstrap(): Promise<Bootstrap> {
  if (!inTauri()) return structuredClone(memory);
  return invoke<Bootstrap>("bootstrap");
}

export async function saveSettings(settings: Settings): Promise<void> {
  if (!inTauri()) {
    memory.settings = settings;
    return;
  }
  await invoke("save_desktop_settings", { settings });
}

export async function saveProviders(store: ProviderStore): Promise<ProviderStore> {
  if (!inTauri()) {
    memory.providers = store;
    memory.settings.active_provider = store.active;
    return store;
  }
  return invoke<ProviderStore>("save_providers", { store });
}

export async function setSecret(id: string, secret: string): Promise<"keyring" | "file"> {
  if (!inTauri()) {
    const status = memory.secrets.find((item) => item.id === id);
    if (status) status.location = "file";
    return "file";
  }
  return invoke("set_secret", { id, secret });
}

export async function clearSecret(id: string): Promise<void> {
  if (!inTauri()) {
    const status = memory.secrets.find((item) => item.id === id);
    if (status) status.location = null;
    return;
  }
  await invoke("clear_secret", { id });
}

export async function createChat(cwd: string, providerId: string, model: string, lang: string): Promise<Session> {
  if (!inTauri()) {
    const session: Session = {
      id: crypto.randomUUID(),
      title: lang.startsWith("zh") ? "新会话" : "New chat",
      cwd,
      provider_id: providerId,
      model,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      messages: [],
    };
    memorySessions.set(session.id, session);
    memory.sessions.unshift(summary(session));
    return session;
  }
  return invoke("create_chat", { cwd, providerId, model, lang });
}

export async function openChat(id: string): Promise<Session> {
  if (!inTauri()) {
    const session = memorySessions.get(id);
    if (!session) throw new Error("missing session");
    return structuredClone(session);
  }
  return invoke("open_chat", { id });
}

export async function removeChat(id: string): Promise<void> {
  if (!inTauri()) {
    memorySessions.delete(id);
    memory.sessions = memory.sessions.filter((session) => session.id !== id);
    return;
  }
  await invoke("remove_chat", { id });
}

export async function sendPrompt(
  sessionId: string,
  text: string,
  onEvent: (event: AgentEvent) => void,
): Promise<Session> {
  if (!inTauri()) {
    onEvent({ type: "reasoning_delta", text: "Browser preview has no model connected. " });
    onEvent({
      type: "text_delta",
      text: "This preview is running outside the desktop shell, so the reply is local. Launch the Tauri app to call DeepSeek, Kimi, OpenAI, or Claude.",
    });
    onEvent({ type: "done" });
    const session = memorySessions.get(sessionId);
    if (!session) throw new Error("missing session");
    session.messages.push({ id: crypto.randomUUID(), role: "user", text, reasoning: "", tool_calls: [] });
    session.messages.push({
      id: crypto.randomUUID(),
      role: "assistant",
      text: "This preview is running outside the desktop shell, so the reply is local. Launch the Tauri app to call DeepSeek, Kimi, OpenAI, or Claude.",
      reasoning: "Browser preview has no model connected. ",
      tool_calls: [],
    });
    session.updated_at = new Date().toISOString();
    return structuredClone(session);
  }
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<AgentEnvelope>("agent-event", (payload) => {
    if (payload.payload.session_id === sessionId) onEvent(payload.payload.event);
  });
  try {
    return await invoke<Session>("send_prompt", { sessionId, text });
  } finally {
    unlisten();
  }
}

export async function cancelPrompt(): Promise<void> {
  if (!inTauri()) return;
  await invoke("cancel_prompt");
}

export async function resolveApproval(id: string, approved: boolean): Promise<void> {
  if (!inTauri()) return;
  await invoke("resolve_approval", { id, approved });
}

export async function testProvider(id: string): Promise<string> {
  if (!inTauri()) return "pong";
  return invoke("test_provider", { id });
}

export async function startTerminal(cols: number, rows: number): Promise<string> {
  if (!inTauri()) throw new Error("The classic TUI runs inside the desktop app.");
  return invoke("start_terminal", { cols, rows });
}

export async function writeTerminal(data: string): Promise<void> {
  await invoke("write_terminal", { data });
}

export async function resizeTerminal(cols: number, rows: number): Promise<void> {
  await invoke("resize_terminal", { cols, rows });
}

export async function stopTerminal(): Promise<void> {
  if (!inTauri()) return;
  await invoke("stop_terminal");
}

export function desktopShell(): boolean {
  return inTauri();
}
