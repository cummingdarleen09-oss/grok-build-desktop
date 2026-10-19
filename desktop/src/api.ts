import type { Lang } from "./i18n";

export type ProviderFormat = "openai" | "anthropic" | "responses";

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
  models: string[];
  follow_latest: boolean;
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
  known_projects: string[];
  theme: "light" | "dark";
}

export interface Automation {
  id: string;
  title: string;
  prompt: string;
  enabled: boolean;
  cwd: string;
  provider_id: string;
  model: string;
  frequency: "daily" | "weekly" | "weekdays" | "once";
  time: string;
  weekdays: number[];
  last_run_at: string;
  next_run_at: string;
}

export interface MediaFile {
  path: string;
  name: string;
  kind: "image" | "video" | "audio" | "pdf" | string;
  preview?: string;
}

export interface GitInfo {
  current: string;
  branches: string[];
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
  parent_session_id?: string | null;
  branched_from_message_id?: string | null;
  branched_at_index?: number | null;
}

export interface SessionSummary {
  id: string;
  title: string;
  model: string;
  provider_id: string;
  cwd: string;
  updated_at: string;
  parent_session_id?: string | null;
  branched_from_message_id?: string | null;
  branched_at_index?: number | null;
  origin: "desktop" | "classic";
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
  from_session: boolean;
}

export interface Bootstrap {
  settings: Settings;
  providers: ProviderStore;
  sessions: SessionSummary[];
  extensions: ExtensionRecord[];
  secrets: SecretStatus[];
  grok_home: string;
  user_name: string;
  automations: Automation[];
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
    preset("xai", "Grok", "responses", "https://api.x.ai/v1", "grok-4.7", "XAI_API_KEY", "bearer", "max_tokens", 500000, ["grok-4.7", "grok-4.6", "grok-4.5"], true, true),
    { ...preset("deepseek", "DeepSeek", "openai", "https://api.deepseek.com", "deepseek-chat", "DEEPSEEK_API_KEY", "bearer", "max_tokens", 128000, ["deepseek-chat", "deepseek-reasoner"], true, false), builtin: false },
    { ...preset("relay", "Work relay", "responses", "https://relay.example/v1", "gpt-5", "RELAY_API_KEY", "bearer", "max_tokens", 128000, ["gpt-5", "gpt-5-mini"], true, false), builtin: false, name: "Work relay" },
  ];
  const project = "/work/bohutongwxweb";
  const ads = "/work/tansuo/adspower";
  const notes = "/work/破限工具测试";
  const session: Session = {
    id: "demo",
    title: "Read the provider layer",
    cwd: project,
    provider_id: "deepseek",
    model: "deepseek-chat",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    messages: [
      msg("m1", "user", "Where do DeepSeek requests get their URL?"),
      {
        id: "m2",
        role: "assistant",
        text: "DeepSeek uses the OpenAI-compatible root https://api.deepseek.com, and the client appends chat/completions.",
        reasoning: "Check the preset table, then the endpoint joiner.",
        tool_calls: [
          {
            id: "t1",
            name: "search_replace",
            arguments: '{"path":"src/checkout.tsx","old":"return <Cart />","new":"return <Checkout />"}',
            output: "updated src/checkout.tsx",
            is_error: false,
          },
        ],
      },
      msg("m3", "user", "Check the preset table specifically."),
      msg("m4", "assistant", "The preset row sets base_url to https://api.deepseek.com and format to openai."),
      msg("m5", "user", "Does the desktop app call that same client?"),
      msg("m6", "assistant", "Yes. grok-desktop-core sends the transcript through grok-providers.\n\n```mermaid\ngraph TD\n  UI-->Core\n  Core-->Providers\n```"),
    ],
  };
  const branch: Session = {
    id: "branch-demo",
    title: "分支 · Read the provider layer",
    cwd: project,
    provider_id: "deepseek",
    model: "deepseek-chat",
    created_at: new Date().toISOString(),
    updated_at: new Date(Date.now() + 1000).toISOString(),
    parent_session_id: "demo",
    branched_from_message_id: "m3",
    branched_at_index: 3,
    messages: session.messages.slice(0, 3).map((message) => ({ ...message, tool_calls: message.tool_calls.map((tool) => ({ ...tool })) })),
  };
  const adsChat: Session = {
    id: "ads",
    title: "Investigate login timeout",
    cwd: ads,
    provider_id: "deepseek",
    model: "deepseek-chat",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    messages: [msg("a1", "user", "The login request hangs after the second redirect.")],
  };
  const scratch: Session = {
    id: "notes",
    title: "对照权限提示文案",
    cwd: notes,
    provider_id: "relay",
    model: "gpt-5",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    messages: [msg("n1", "user", "Keep a short list of follow-ups for this folder.")],
  };
  memorySessions.set(session.id, session);
  memorySessions.set(branch.id, branch);
  memorySessions.set(adsChat.id, adsChat);
  memorySessions.set(scratch.id, scratch);
  const classic: SessionSummary = {
    id: "classic-demo",
    title: "TUI · 旧会话",
    model: "grok-4",
    provider_id: "classic",
    cwd: project,
    updated_at: new Date().toISOString(),
    origin: "classic",
  };
  memorySessions.set("classic-demo", {
    id: "classic-demo",
    title: "TUI · 旧会话",
    cwd: project,
    provider_id: "classic",
    model: "grok-4",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    messages: [msg("c1", "user", "Where is the config?"), msg("c2", "assistant", "It lives in config.toml.")],
  });
  return {
    settings: {
      lang: "zh-CN",
      permission: "ask",
      cwd: project,
      active_provider: "xai",
      disabled_tools: [],
      known_projects: [project, ads, notes],
      theme: "light",
    },
    providers: { active: "xai", providers },
    sessions: [summary(branch), summary(session), summary(adsChat), summary(scratch), classic],
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
      location: null,
      from_env: false,
      from_session: provider.id === "xai",
    })),
    grok_home: "~/.grok",
    user_name: "dev",
    automations: [
      {
        id: "auto-1",
        title: "Weekday plan",
        prompt: "Summarize the git diff and propose the next step.",
        enabled: true,
        cwd: project,
        provider_id: "xai",
        model: "grok-4.7",
        frequency: "weekdays",
        time: "09:00",
        weekdays: [1, 2, 3, 4, 5],
        last_run_at: "",
        next_run_at: "",
      },
    ],
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
  follow_latest: boolean,
): Provider {
  return {
    id,
    name,
    format,
    base_url,
    model,
    env_key,
    env_key_fallbacks: id === "kimi" ? ["KIMI_API_KEY"] : id === "xai" ? ["GROK_CODE_XAI_API_KEY"] : [],
    context_window,
    extra_headers: format === "anthropic" ? { "anthropic-version": "2023-06-01" } : {},
    env_http_headers: format === "anthropic" ? { "x-api-key": "ANTHROPIC_API_KEY" } : {},
    auth,
    enabled,
    temperature: null,
    max_tokens: 8192,
    token_field,
    builtin: id === "xai",
    model_hints,
    models: model_hints.includes(model) ? model_hints : [model, ...model_hints],
    follow_latest,
  };
}

export function providerModels(provider: Provider): string[] {
  const source = provider.models?.length ? provider.models : provider.model_hints ?? [];
  const models: string[] = [];
  for (const id of [...source, provider.model]) {
    const trimmed = id?.trim();
    if (trimmed && !models.includes(trimmed)) models.push(trimmed);
  }
  return models;
}

export const providerTemplates: Array<Pick<Provider, "id" | "name" | "format" | "base_url" | "model" | "env_key" | "auth" | "token_field" | "context_window" | "models">> = [
  { id: "deepseek", name: "DeepSeek", format: "openai", base_url: "https://api.deepseek.com", model: "deepseek-chat", env_key: "DEEPSEEK_API_KEY", auth: "bearer", token_field: "max_tokens", context_window: 128000, models: ["deepseek-chat", "deepseek-reasoner"] },
  { id: "kimi", name: "Moonshot Kimi", format: "openai", base_url: "https://api.moonshot.cn/v1", model: "moonshot-v1-128k", env_key: "MOONSHOT_API_KEY", auth: "bearer", token_field: "max_tokens", context_window: 128000, models: ["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k", "kimi-latest"] },
  { id: "openai", name: "OpenAI", format: "openai", base_url: "https://api.openai.com/v1", model: "gpt-4o", env_key: "OPENAI_API_KEY", auth: "bearer", token_field: "max_completion_tokens", context_window: 128000, models: ["gpt-4o", "gpt-4.1"] },
  { id: "anthropic", name: "Anthropic Claude", format: "anthropic", base_url: "https://api.anthropic.com/v1", model: "claude-sonnet-4-5", env_key: "ANTHROPIC_API_KEY", auth: "x_api_key", token_field: "max_tokens", context_window: 200000, models: ["claude-sonnet-4-5", "claude-opus-4-5"] },
];

function msg(id: string, role: "user" | "assistant", text: string): ChatMessage {
  return { id, role, text, reasoning: "", tool_calls: [] };
}

function summary(session: Session): SessionSummary {
  return {
    id: session.id,
    title: session.title,
    model: session.model,
    provider_id: session.provider_id,
    cwd: session.cwd,
    updated_at: session.updated_at,
    parent_session_id: session.parent_session_id ?? null,
    branched_from_message_id: session.branched_from_message_id ?? null,
    branched_at_index: session.branched_at_index ?? null,
    origin: "desktop",
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
    if (status) {
      status.location = "file";
      status.from_session = false;
    }
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
    remember(session);
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

export async function renameChat(id: string, title: string): Promise<Session> {
  if (!inTauri()) {
    const session = memorySessions.get(id);
    if (!session) throw new Error("missing session");
    session.title = title.trim();
    session.updated_at = new Date().toISOString();
    remember(session);
    return structuredClone(session);
  }
  return invoke<Session>("rename_chat", { id, title });
}

export async function branchChat(
  sessionId: string,
  messageId: string,
  includeMessage: boolean,
  lang: string,
): Promise<Session> {
  if (!inTauri()) {
    const parent = memorySessions.get(sessionId);
    if (!parent) throw new Error("missing session");
    const index = parent.messages.findIndex((message) => message.id === messageId);
    if (index < 0) throw new Error("message not found");
    if (!includeMessage && parent.messages[index].role !== "user") {
      throw new Error("only a user message can be edited into a new branch");
    }
    const end = includeMessage ? index + 1 : index;
    const messages = parent.messages.slice(0, end).map((message) => ({
      ...message,
      tool_calls: message.tool_calls.map((tool) => ({ ...tool })),
    }));
    const last = messages[messages.length - 1];
    if (last && last.role === "assistant") {
      last.tool_calls = last.tool_calls.filter((tool) => tool.output != null);
      if (!last.text.trim() && !last.reasoning.trim() && last.tool_calls.length === 0) messages.pop();
    }
    const session: Session = {
      id: crypto.randomUUID(),
      title: `${lang.startsWith("zh") ? "分支" : "Branch"} · ${parent.title}`,
      cwd: parent.cwd,
      provider_id: parent.provider_id,
      model: parent.model,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      messages,
      parent_session_id: parent.id,
      branched_from_message_id: messageId,
      branched_at_index: index + 1,
    };
    remember(session);
    return structuredClone(session);
  }
  return invoke<Session>("branch_chat", {
    sessionId,
    messageId,
    includeMessage,
    lang,
  });
}

export async function pickProjectFolder(): Promise<string | null> {
  if (!inTauri()) {
    const typed = window.prompt("Project folder");
    return typed && typed.trim() ? typed.trim() : null;
  }
  try {
    return await invoke<string>("pick_project_folder");
  } catch (error) {
    const message = String(error);
    if (message.includes("cancelled")) return null;
    const typed = window.prompt("Project folder");
    return typed && typed.trim() ? typed.trim() : null;
  }
}

function remember(session: Session) {
  memorySessions.set(session.id, session);
  const next = summary(session);
  const index = memory.sessions.findIndex((item) => item.id === session.id);
  if (index >= 0) memory.sessions[index] = next;
  else memory.sessions.unshift(next);
}

export async function setChatModel(id: string, providerId: string, model: string): Promise<Session> {
  if (!inTauri()) {
    const session = memorySessions.get(id);
    if (!session) throw new Error("missing session");
    session.provider_id = providerId;
    session.model = model;
    remember(session);
    return structuredClone(session);
  }
  return invoke<Session>("set_chat_model", { id, providerId, model });
}

export async function fetchModels(id: string, baseUrl: string, format: ProviderFormat, apiKey: string): Promise<string[]> {
  if (!inTauri()) {
    if (!baseUrl.trim()) throw new Error("base URL is required");
    if (format === "anthropic") return ["claude-sonnet-4-5", "claude-opus-4-5"];
    if (format === "responses") return ["gpt-5", "gpt-5-mini", "grok-4.7"];
    return ["deepseek-chat", "deepseek-reasoner"];
  }
  return invoke<string[]>("fetch_models", { id, baseUrl, format, apiKey });
}

export async function sendPrompt(
  sessionId: string,
  text: string,
  onEvent: (event: AgentEvent) => void,
  plan = false,
  model?: string,
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
    if (session.title === "新会话" || session.title === "New chat") {
      session.title = text.slice(0, 48);
    }
    remember(session);
    return structuredClone(session);
  }
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<AgentEnvelope>("agent-event", (payload) => {
    if (payload.payload.session_id === sessionId) onEvent(payload.payload.event);
  });
  try {
    return await invoke<Session>("send_prompt", { sessionId, text, plan, model });
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

export async function testProvider(id: string, model?: string): Promise<string> {
  if (!inTauri()) return "pong";
  return invoke("test_provider", { id, model });
}

export async function openClassic(id: string): Promise<Session> {
  if (!inTauri()) return openChat(id);
  return invoke("open_classic", { id });
}

export async function renderDiagram(source: string, dark: boolean): Promise<string | null> {
  if (!inTauri()) return null;
  try {
    return await invoke<string>("render_diagram", { source, dark });
  } catch {
    return null;
  }
}

export async function projectMedia(cwd: string): Promise<MediaFile[]> {
  if (!inTauri()) {
    return [
      { path: `${cwd}/design/checkout.png`, name: "checkout.png", kind: "image", preview: "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="100%" height="100%" fill="#f4f4f5"/><rect x="24" y="28" width="272" height="124" rx="12" fill="#fff" stroke="#e4e4e7"/><text x="40" y="96" font-family="sans-serif" font-size="16" fill="#3f3f46">checkout.png</text></svg>') },
      { path: `${cwd}/notes/spec.pdf`, name: "spec.pdf", kind: "pdf" },
    ];
  }
  return invoke("project_media", { cwd });
}

export async function loadAutomations(): Promise<Automation[]> {
  if (!inTauri()) return structuredClone(memory.automations);
  return invoke("load_automation_list");
}

export async function storeAutomations(list: Automation[]): Promise<Automation[]> {
  if (!inTauri()) {
    memory.automations = list;
    return list;
  }
  return invoke("store_automations", { list });
}

export async function gitInfo(cwd: string): Promise<GitInfo> {
  if (!inTauri()) return { current: "main", branches: ["main", "feat/checkout"] };
  return invoke("git_info", { cwd });
}

export function desktopShell(): boolean {
  return inTauri();
}
