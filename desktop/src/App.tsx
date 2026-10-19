import { useEffect, useMemo, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import {
  type AgentEvent,
  type Bootstrap,
  type ExtensionRecord,
  type Provider,
  type ProviderStore,
  type Session,
  type SessionSummary,
  type Settings,
  branchChat,
  cancelPrompt,
  clearSecret,
  createChat,
  desktopShell,
  loadBootstrap,
  openChat,
  pickProjectFolder,
  removeChat,
  renameChat,
  resolveApproval,
  resizeTerminal,
  saveProviders,
  saveSettings,
  sendPrompt,
  setSecret,
  startTerminal,
  stopTerminal,
  testProvider,
  writeTerminal,
} from "./api";
import { branchBanner, desktopShortcuts, translate, tuiShortcuts, type Key, type Lang } from "./i18n";

type Page = "chat" | "extensions" | "terminal" | "shortcuts" | "settings";

const TOOLS = ["read_file", "write_file", "search_replace", "grep", "list_dir", "run_terminal"] as const;

export function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [page, setPage] = useState<Page>("chat");
  const [session, setSession] = useState<Session | null>(null);
  const [classic, setClassic] = useState<SessionSummary | null>(null);
  const [focusMessageId, setFocusMessageId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const lang: Lang = boot?.settings.lang === "en" ? "en" : "zh-CN";
  const t = (key: Key) => translate(lang, key);

  async function refresh(openId?: string) {
    const next = await loadBootstrap();
    setBoot(next);
    const preview = !desktopShell() ? "branch-demo" : undefined;
    const id = openId ?? session?.id ?? preview;
    if (id && next.sessions.some((item) => item.id === id && item.origin !== "classic")) {
      setClassic(null);
      setSession(await openChat(id));
    } else if (!session && next.sessions.find((item) => item.origin !== "classic") && !desktopShell()) {
      const fallback = next.sessions.find((item) => item.origin !== "classic");
      if (fallback) setSession(await openChat(fallback.id));
    }
  }

  useEffect(() => {
    refresh().catch((reason: unknown) => setError(String(reason)));
    // The first load is the only automatic fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (!mod) return;
      const pages: Page[] = ["chat", "extensions", "terminal", "shortcuts", "settings"];
      const index = Number(event.key) - 1;
      if (index >= 0 && index < pages.length) {
        event.preventDefault();
        setPage(pages[index]);
      } else if (event.key === ",") {
        event.preventDefault();
        setPage("settings");
      } else if (event.key.toLowerCase() === "n") {
        event.preventDefault();
        void newChat();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  async function newChat() {
    if (!boot) return;
    const provider = boot.providers.providers.find((item) => item.id === boot.settings.active_provider) ?? boot.providers.providers.find((item) => item.enabled);
    if (!provider) {
      setPage("settings");
      return;
    }
    const created = await createChat(boot.settings.cwd, provider.id, provider.model, lang);
    setClassic(null);
    setSession(created);
    setPage("chat");
    await refresh(created.id);
  }

  async function addProject() {
    if (!boot) return;
    const picked = await pickProjectFolder();
    if (!picked) return;
    const path = projectPath(picked);
    const known = boot.settings.known_projects.some((item) => projectPath(item) === path)
      ? boot.settings.known_projects
      : [...boot.settings.known_projects, path];
    const settings = { ...boot.settings, cwd: path, known_projects: known };
    await saveSettings(settings);
    setBoot({ ...boot, settings });
    setClassic(null);
    setSession(null);
    setPage("chat");
  }

  async function selectProject(path: string) {
    if (!boot) return;
    const settings = { ...boot.settings, cwd: path };
    await saveSettings(settings);
    setBoot({ ...boot, settings });
    setPage("chat");
    const first = boot.sessions.find((item) => projectPath(item.cwd) === projectPath(path) && item.origin !== "classic");
    setClassic(null);
    setSession(first ? await openChat(first.id) : null);
  }

  async function openListed(item: SessionSummary) {
    if (!boot) return;
    const settings = { ...boot.settings, cwd: projectPath(item.cwd) };
    await saveSettings(settings);
    setBoot({ ...boot, settings });
    setPage("chat");
    setFocusMessageId(null);
    if (item.origin === "classic") {
      setSession(null);
      setClassic(item);
      return;
    }
    setClassic(null);
    setSession(await openChat(item.id));
  }

  async function openParent(id: string, messageId?: string | null) {
    setClassic(null);
    setFocusMessageId(messageId ?? null);
    setSession(await openChat(id));
    setPage("chat");
  }

  async function setLang(next: Lang) {
    if (!boot) return;
    const settings = { ...boot.settings, lang: next };
    await saveSettings(settings);
    setBoot({ ...boot, settings });
  }

  if (!boot) {
    return <div className="content">{error ?? "…"}</div>;
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="mark">G</div>
          <div>
            <strong>{t("app.title")}</strong>
            <span>{t("app.subtitle")}</span>
          </div>
        </div>
        <nav className="nav">
          <Nav page="chat" current={page} setPage={setPage} label={t("nav.chat")} />
          <Nav page="extensions" current={page} setPage={setPage} label={t("nav.extensions")} />
          <Nav page="terminal" current={page} setPage={setPage} label={t("nav.terminal")} />
          <Nav page="shortcuts" current={page} setPage={setPage} label={t("nav.shortcuts")} />
          <Nav page="settings" current={page} setPage={setPage} label={t("nav.settings")} />
        </nav>
        <button className="ghost" onClick={() => void addProject()}>{t("project.add")}</button>
        <button className="primary" onClick={() => void newChat()}>{t("chat.new")}</button>
        <div className="session-list">
          {projectGroups(boot).map((group) => (
            <section key={group.path} className={projectPath(boot.settings.cwd) === group.path ? "project active" : "project"}>
              <button className="project-head" onClick={() => void selectProject(group.path).catch((reason: unknown) => setError(String(reason)))}>
                <strong>{projectLeaf(group.path)}</strong>
                <span title={group.path}>{group.path}</span>
              </button>
              {group.sessions.length === 0 && <p className="project-empty">{t("project.empty")}</p>}
              {group.sessions.map((item) => (
                <ConversationRow
                  key={`${item.origin}-${item.id}`}
                  item={item}
                  active={session?.id === item.id || classic?.id === item.id}
                  renameLabel={t("chat.rename")}
                  onOpen={() => void openListed(item).catch((reason: unknown) => setError(String(reason)))}
                  onRename={async (title) => {
                    const next = await renameChat(item.id, title);
                    setSession(next);
                    await refresh(next.id);
                  }}
                />
              ))}
            </section>
          ))}
        </div>
      </aside>
      <section className="main">
        <header className="topbar">
          <div>
            <strong>{page === "chat" ? session?.title ?? t("nav.chat") : t(pageTitle(page))}</strong>
            <div className="meta">{boot.settings.cwd}</div>
          </div>
          <div className="lang">
            <button className={lang === "zh-CN" ? "active" : ""} onClick={() => void setLang("zh-CN")}>中文</button>
            <button className={lang === "en" ? "active" : ""} onClick={() => void setLang("en")}>EN</button>
          </div>
        </header>
        {error && <div className="banner" onClick={() => setError(null)}>{error}</div>}
        {page === "chat" && classic && (
          <div className="content">
            <div className="empty">
              <h1>{classic.title}</h1>
              <p>{t("project.classicBody")}</p>
              <button className="primary" onClick={() => setPage("terminal")}>{t("project.classic")}</button>
            </div>
          </div>
        )}
        {page === "chat" && !classic && (
          <Chat
            t={t}
            lang={lang}
            boot={boot}
            session={session}
            busy={busy}
            focusMessageId={focusMessageId}
            setBusy={setBusy}
            setError={setError}
            onOpen={(id, messageId) => void openParent(id, messageId).catch((reason: unknown) => setError(String(reason)))}
            onBranch={async (messageId, includeMessage, replacement) => {
              if (!session) return;
              const created = await branchChat(session.id, messageId, includeMessage, lang);
              setSession(created);
              setFocusMessageId(null);
              if (replacement?.trim()) {
                setBusy(true);
                try {
                  const next = await sendPrompt(created.id, replacement.trim(), () => undefined);
                  setSession(next);
                  await refresh(next.id);
                } finally {
                  setBusy(false);
                }
              } else {
                await refresh(created.id);
              }
            }}
            onSession={async (next) => {
              setSession(next);
              const listed = await loadBootstrap();
              setBoot(listed);
            }}
            onDelete={async (id) => {
              await removeChat(id);
              setSession(null);
              await refresh();
            }}
          />
        )}
        {page === "settings" && (
          <SettingsView
            t={t}
            boot={boot}
            onSaved={(settings, providers) => setBoot({ ...boot, settings, providers })}
            setError={setError}
          />
        )}
        {page === "extensions" && (
          <Extensions t={t} records={boot.extensions} settings={boot.settings} onSettings={(settings) => setBoot({ ...boot, settings })} onOpenTerminal={() => setPage("terminal")} setError={setError} />
        )}
        {page === "terminal" && <ClassicTerminal t={t} binary={boot.grok_binary} />}
        {page === "shortcuts" && <Shortcuts t={t} lang={lang} />}
      </section>
    </div>
  );
}

function pageTitle(page: Page): Key {
  if (page === "extensions") return "nav.extensions";
  if (page === "terminal") return "nav.terminal";
  if (page === "shortcuts") return "nav.shortcuts";
  if (page === "settings") return "nav.settings";
  return "nav.chat";
}

function projectPath(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, "");
  return trimmed || path.trim();
}

function projectLeaf(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

function projectGroups(boot: Bootstrap): { path: string; sessions: SessionSummary[] }[] {
  const paths = new Map<string, string>();
  const add = (path: string) => {
    const key = projectPath(path);
    if (key) paths.set(key, key);
  };
  add(boot.settings.cwd);
  for (const path of boot.settings.known_projects ?? []) add(path);
  for (const session of boot.sessions) add(session.cwd);
  return [...paths.values()]
    .sort((left, right) => projectLeaf(left).localeCompare(projectLeaf(right)))
    .map((path) => ({
      path,
      sessions: boot.sessions
        .filter((session) => projectPath(session.cwd) === path)
        .sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    }));
}

function parentTitle(boot: Bootstrap, id: string): string {
  return boot.sessions.find((session) => session.id === id)?.title ?? id;
}

function ConversationRow(props: {
  item: SessionSummary;
  active: boolean;
  renameLabel: string;
  onOpen: () => void;
  onRename: (title: string) => Promise<void>;
}) {
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState(props.item.title);
  return (
    <div className={props.active ? "conversation active" : "conversation"}>
      {renaming ? (
        <form onSubmit={(event) => {
          event.preventDefault();
          setRenaming(false);
          void props.onRename(title);
        }}>
          <input value={title} onChange={(event) => setTitle(event.target.value)} autoFocus />
        </form>
      ) : (
        <button onClick={props.onOpen}>
          <span>{props.item.origin === "classic" ? "TUI · " : props.item.parent_session_id ? "↳ " : ""}{props.item.title}</span>
          <em>{props.item.origin === "classic" ? "classic" : props.item.model}</em>
        </button>
      )}
      {props.item.origin !== "classic" && !renaming && (
        <button className="icon" title={props.renameLabel} onClick={() => { setTitle(props.item.title); setRenaming(true); }}>✎</button>
      )}
    </div>
  );
}

function Nav(props: { page: Page; current: Page; setPage: (page: Page) => void; label: string }) {
  return (
    <button className={props.current === props.page ? "active" : ""} onClick={() => props.setPage(props.page)}>
      {props.label}
    </button>
  );
}

function Chat(props: {
  t: (key: Key) => string;
  lang: Lang;
  boot: Bootstrap;
  session: Session | null;
  busy: boolean;
  focusMessageId: string | null;
  setBusy: (busy: boolean) => void;
  setError: (error: string | null) => void;
  onOpen: (id: string, messageId?: string | null) => void;
  onBranch: (messageId: string, includeMessage: boolean, replacement?: string) => Promise<void>;
  onSession: (session: Session) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}) {
  const { t, session } = props;
  const [draftText, setDraftText] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const [live, setLive] = useState("");
  const [reasoning, setReasoning] = useState("");
  const [tools, setTools] = useState<Session["messages"][number]["tool_calls"]>([]);
  const [approval, setApproval] = useState<{ id: string; name: string; arguments: string } | null>(null);
  const box = useRef<HTMLTextAreaElement>(null);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (props.focusMessageId) {
      document.getElementById(`message-${props.focusMessageId}`)?.scrollIntoView({ block: "center" });
      return;
    }
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [session, live, tools, approval, props.focusMessageId]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        box.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  async function send(text: string) {
    if (!session || !text.trim() || props.busy) return;
    setDraftText("");
    setLive("");
    setReasoning("");
    setTools([]);
    props.setBusy(true);
    props.setError(null);
    const onEvent = (event: AgentEvent) => {
      if (event.type === "text_delta") setLive((value) => value + event.text);
      if (event.type === "reasoning_delta") setReasoning((value) => value + event.text);
      if (event.type === "approval_needed") setApproval({ id: event.id, name: event.name, arguments: event.arguments });
      if (event.type === "tool_started") {
        setApproval(null);
        setTools((current) => [
          ...current.filter((tool) => tool.id !== event.id),
          { id: event.id, name: event.name, arguments: event.arguments, output: null, is_error: false },
        ]);
      }
      if (event.type === "tool_finished") {
        setTools((current) => current.map((tool) => tool.id === event.id ? { ...tool, output: event.output, is_error: event.is_error } : tool));
      }
      if (event.type === "error") props.setError(event.message);
    };
    try {
      const next = await sendPrompt(session.id, text.trim(), onEvent);
      setLive("");
      setReasoning("");
      setTools([]);
      setApproval(null);
      await props.onSession(next);
    } catch (reason) {
      props.setError(String(reason));
    } finally {
      props.setBusy(false);
    }
  }

  return (
    <>
      <div className="content" ref={scroller}>
        {!session && (
          <div className="empty">
            <h1>{t("chat.emptyTitle")}</h1>
            <p>{t("chat.emptyBody")}</p>
          </div>
        )}
        {session && session.messages.length === 0 && !props.busy && (
          <div className="empty">
            <h1>{t("chat.emptyTitle")}</h1>
            <p>{t("chat.emptyBody")}</p>
            <div className="chips">
              {(["chat.suggest1", "chat.suggest2", "chat.suggest3"] as const).map((key) => (
                <button key={key} onClick={() => setDraftText(t(key))}>{t(key)}</button>
              ))}
            </div>
          </div>
        )}
        {session?.parent_session_id && session.branched_at_index && (
          <button
            className="branch-banner"
            onClick={() => props.onOpen(session.parent_session_id || "", session.branched_from_message_id)}
          >
            {branchBanner(props.lang, parentTitle(props.boot, session.parent_session_id), session.branched_at_index)}
          </button>
        )}
        <div className="transcript">
          {session?.messages.map((message, index) => {
            const branches = props.boot.sessions.filter((item) => item.parent_session_id === session.id && item.branched_from_message_id === message.id);
            return (
              <article key={message.id} id={`message-${message.id}`} className={`bubble ${message.role} ${props.focusMessageId === message.id ? "focused" : ""}`}>
                <div className="who">{message.role === "user" ? t("chat.you") : session.model} <span>#{index + 1}</span></div>
                {message.reasoning && <div className="reasoning">{message.reasoning}</div>}
                {editingId === message.id ? (
                  <textarea className="edit-box" value={editText} onChange={(event) => setEditText(event.target.value)} />
                ) : (
                  message.text && <div className="text">{message.text}</div>
                )}
                {message.tool_calls.map((tool) => <ToolCard key={tool.id} t={t} tool={tool} />)}
                <div className="msg-actions">
                  <button type="button" className="ghost" disabled={props.busy} onClick={() => void props.onBranch(message.id, true).catch((reason: unknown) => props.setError(String(reason)))}>{t("chat.branch")}</button>
                  {message.role === "user" && editingId !== message.id && (
                    <button type="button" className="ghost" disabled={props.busy} onClick={() => { setEditingId(message.id); setEditText(message.text); }}>{t("chat.edit")}</button>
                  )}
                  {editingId === message.id && (
                    <button type="button" className="primary" disabled={props.busy || !editText.trim()} onClick={() => {
                      const text = editText;
                      setEditingId(null);
                      void props.onBranch(message.id, false, text).catch((reason: unknown) => props.setError(String(reason)));
                    }}>{t("chat.resend")}</button>
                  )}
                </div>
                {branches.length > 0 && (
                  <div className="branch-list">
                    <span>{t("chat.branches")}</span>
                    {branches.map((item) => (
                      <button key={item.id} type="button" onClick={() => props.onOpen(item.id)}>{item.title}</button>
                    ))}
                  </div>
                )}
              </article>
            );
          })}
          {props.busy && (
            <article className="bubble assistant">
              <div className="who">{t("chat.thinking")}</div>
              {reasoning && <div className="reasoning">{reasoning}</div>}
              {live && <div className="text">{live}</div>}
              {tools.map((tool) => <ToolCard key={tool.id} t={t} tool={tool} />)}
            </article>
          )}
        </div>
      </div>
      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault();
          void send(draftText);
        }}
      >
        {approval && (
          <div className="approval">
            <div>
              <strong>{t("chat.approval")}</strong>
              <div className="meta">{approval.name} {approval.arguments}</div>
            </div>
            <div className="row">
              <button type="button" className="primary" onClick={() => void resolveApproval(approval.id, true)}>{t("chat.approve")}</button>
              <button type="button" className="danger" onClick={() => void resolveApproval(approval.id, false)}>{t("chat.deny")}</button>
            </div>
          </div>
        )}
        <textarea
          ref={box}
          value={draftText}
          placeholder={t("chat.placeholder")}
          onChange={(event) => setDraftText(event.target.value)}
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
              event.preventDefault();
              void send(draftText);
            }
            if (event.key === "Escape" && props.busy) {
              void cancelPrompt();
            }
          }}
        />
        <div className="row">
          <span className="hint">Ctrl/⌘ + Enter</span>
          <span className="grow" />
          {session && <button type="button" className="ghost" onClick={() => void props.onDelete(session.id)}>✕</button>}
          {props.busy ? (
            <button type="button" className="danger" onClick={() => void cancelPrompt()}>{t("chat.stop")}</button>
          ) : (
            <button className="primary" type="submit" disabled={!session}>{t("chat.send")}</button>
          )}
        </div>
      </form>
    </>
  );
}

function ToolCard(props: { t: (key: Key) => string; tool: Session["messages"][number]["tool_calls"][number] }) {
  const label = TOOLS.includes(props.tool.name as (typeof TOOLS)[number])
    ? props.t(`tool.${props.tool.name}` as Key)
    : props.tool.name;
  return (
    <div className={props.tool.is_error ? "tool error" : "tool"}>
      <header>
        <b>{label}</b>
        <span>{props.tool.name}</span>
      </header>
      <pre>{props.tool.arguments}</pre>
      {props.tool.output && <pre>{props.tool.output}</pre>}
    </div>
  );
}

function SettingsView(props: {
  t: (key: Key) => string;
  boot: Bootstrap;
  onSaved: (settings: Settings, providers: ProviderStore) => void;
  setError: (error: string | null) => void;
}) {
  const { t } = props;
  const [settings, setSettings] = useState(props.boot.settings);
  const [providers, setProviders] = useState(props.boot.providers.providers);
  const [secrets, setSecrets] = useState(props.boot.secrets);
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [notice, setNotice] = useState<string | null>(null);
  const [custom, setCustom] = useState<{ id: string; name: string; format: Provider["format"]; base_url: string; model: string; env_key: string }>({ id: "", name: "", format: "openai", base_url: "http://127.0.0.1:8000/v1", model: "", env_key: "CUSTOM_API_KEY" });

  function update(id: string, patch: Partial<Provider>) {
    setProviders((current) => current.map((provider) => provider.id === id ? { ...provider, ...patch } : provider));
  }

  async function save() {
    props.setError(null);
    const store: ProviderStore = { active: settings.active_provider, providers };
    try {
      await saveSettings(settings);
      const saved = await saveProviders(store);
      setProviders(saved.providers);
      props.onSaved({ ...settings, active_provider: saved.active }, saved);
      setNotice(t("settings.saved"));
    } catch (reason) {
      props.setError(String(reason));
    }
  }

  return (
    <div className="content">
      <div className="page-intro">
        <h1>{t("nav.settings")}</h1>
        <p>{t("settings.providersHint")}</p>
      </div>
      <div className="panel" style={{ marginBottom: 12 }}>
        <div className="row">
          <label className="grow">{t("settings.cwd")}
            <input value={settings.cwd} onChange={(event) => setSettings({ ...settings, cwd: event.target.value })} />
          </label>
          <label>{t("settings.permission")}
            <select value={settings.permission} onChange={(event) => setSettings({ ...settings, permission: event.target.value as Settings["permission"] })}>
              <option value="ask">{t("settings.ask")}</option>
              <option value="auto">{t("settings.auto")}</option>
            </select>
          </label>
        </div>
      </div>
      <div className="cards">
        {providers.map((provider) => {
          const secret = secrets.find((item) => item.id === provider.id);
          const where = secret?.location === "keyring" ? t("settings.keyring") : secret?.location === "file" ? t("settings.file") : secret?.from_session ? t("settings.session") : secret?.from_env ? t("settings.env") : t("settings.none");
          return (
            <article key={provider.id} className="card">
              <h3>{provider.name}</h3>
              <div className="url">{provider.base_url}</div>
              <div className="checks">
                <label><input type="checkbox" checked={provider.enabled} onChange={(event) => update(provider.id, { enabled: event.target.checked })} />{t("settings.enabled")}</label>
                <label><input type="radio" name="active" checked={settings.active_provider === provider.id} onChange={() => setSettings({ ...settings, active_provider: provider.id })} />{t("settings.active")}</label>
                {provider.id === "xai" && (
                  <label><input type="checkbox" checked={provider.follow_latest} onChange={(event) => update(provider.id, { follow_latest: event.target.checked })} />{t("settings.latest")}</label>
                )}
                <span className="status-pill">{where}</span>
              </div>
              {provider.id === "xai" && <p className="hint">{t("settings.latestHint")}</p>}
              <label>{t("settings.baseUrl")}<input value={provider.base_url} onChange={(event) => update(provider.id, { base_url: event.target.value })} /></label>
              <label>{t("settings.model")}
                <input list={`${provider.id}-models`} value={provider.model} onChange={(event) => update(provider.id, provider.id === "xai" ? { model: event.target.value, follow_latest: false } : { model: event.target.value })} />
                <datalist id={`${provider.id}-models`}>{provider.model_hints.map((hint) => <option key={hint} value={hint} />)}</datalist>
              </label>
              <label>{t("settings.context")}<input type="number" value={provider.context_window} onChange={(event) => update(provider.id, { context_window: Number(event.target.value) })} /></label>
              <label>{t("settings.apiKey")}
                <input type="password" autoComplete="off" value={keys[provider.id] ?? ""} placeholder={provider.env_key} onChange={(event) => setKeys({ ...keys, [provider.id]: event.target.value })} />
              </label>
              <div className="row" style={{ marginTop: 10 }}>
                <button type="button" className="ghost" onClick={() => {
                  const secretValue = keys[provider.id] ?? "";
                  void setSecret(provider.id, secretValue).then((location) => {
                    setSecrets((current) => current.map((item) => item.id === provider.id ? { ...item, location } : item));
                    setKeys({ ...keys, [provider.id]: "" });
                  }).catch((reason: unknown) => props.setError(String(reason)));
                }}>{t("settings.saveKey")}</button>
                <button type="button" className="ghost" onClick={() => {
                  void clearSecret(provider.id).then(() => {
                    setSecrets((current) => current.map((item) => item.id === provider.id ? { ...item, location: null } : item));
                  }).catch((reason: unknown) => props.setError(String(reason)));
                }}>{t("settings.clearKey")}</button>
                <button type="button" className="ghost" onClick={() => {
                  void testProvider(provider.id).then((text) => setNotice(text || "ok")).catch((reason: unknown) => props.setError(String(reason)));
                }}>{t("settings.test")}</button>
              </div>
            </article>
          );
        })}
        <article className="card">
          <h3>{t("settings.custom")}</h3>
          <label>{t("settings.id")}<input value={custom.id} onChange={(event) => setCustom({ ...custom, id: event.target.value })} /></label>
          <label>{t("settings.name")}<input value={custom.name} onChange={(event) => setCustom({ ...custom, name: event.target.value })} /></label>
          <label>{t("settings.format")}
            <select value={custom.format} onChange={(event) => setCustom({ ...custom, format: event.target.value as Provider["format"] })}>
              <option value="openai">{t("settings.openai")}</option>
              <option value="anthropic">{t("settings.anthropic")}</option>
              <option value="responses">{t("settings.responses")}</option>
            </select>
          </label>
          <label>{t("settings.baseUrl")}<input value={custom.base_url} onChange={(event) => setCustom({ ...custom, base_url: event.target.value })} /></label>
          <label>{t("settings.model")}<input value={custom.model} onChange={(event) => setCustom({ ...custom, model: event.target.value })} /></label>
          <label>{t("settings.envKey")}<input value={custom.env_key} onChange={(event) => setCustom({ ...custom, env_key: event.target.value })} /></label>
          <button type="button" className="primary" style={{ marginTop: 12 }} onClick={() => {
            if (!custom.id || !custom.model) return;
            const provider: Provider = {
              id: custom.id,
              name: custom.name || custom.id,
              format: custom.format,
              base_url: custom.base_url,
              model: custom.model,
              env_key: custom.env_key,
              env_key_fallbacks: [],
              context_window: custom.format === "anthropic" ? 200000 : 128000,
              extra_headers: custom.format === "anthropic" ? { "anthropic-version": "2023-06-01" } : {},
              env_http_headers: custom.format === "anthropic" ? { "x-api-key": custom.env_key } : {},
              auth: custom.format === "anthropic" ? "x_api_key" : "bearer",
              enabled: true,
              temperature: null,
              max_tokens: 8192,
              token_field: "max_tokens",
              builtin: false,
              model_hints: [custom.model],
              follow_latest: false,
            };
            setProviders((current) => [...current.filter((item) => item.id !== provider.id), provider]);
            setSecrets((current) => current.some((item) => item.id === provider.id) ? current : [...current, { id: provider.id, location: null, from_env: false, from_session: false }]);
          }}>{t("settings.add")}</button>
        </article>
      </div>
      <div className="row" style={{ marginTop: 14 }}>
        <button className="primary" onClick={() => void save()}>{t("settings.save")}</button>
        {notice && <span className="hint">{notice}</span>}
      </div>
    </div>
  );
}

function Extensions(props: {
  t: (key: Key) => string;
  records: ExtensionRecord[];
  settings: Settings;
  onSettings: (settings: Settings) => void;
  onOpenTerminal: () => void;
  setError: (error: string | null) => void;
}) {
  const { t } = props;
  const discovered = props.records.filter((record) => record.kind !== "tool");
  function toggle(tool: string) {
    const disabled = props.settings.disabled_tools.includes(tool)
      ? props.settings.disabled_tools.filter((item) => item !== tool)
      : [...props.settings.disabled_tools, tool];
    const settings = { ...props.settings, disabled_tools: disabled };
    void saveSettings(settings).then(() => props.onSettings(settings)).catch((reason: unknown) => props.setError(String(reason)));
  }
  return (
    <div className="content">
      <div className="page-intro">
        <h1>{t("ext.title")}</h1>
        <p>{t("ext.body")}</p>
      </div>
      <h2>{t("ext.tools")}</h2>
      <div className="cards">
        {TOOLS.map((tool) => (
          <label key={tool} className="card checks">
            <input type="checkbox" checked={!props.settings.disabled_tools.includes(tool)} onChange={() => toggle(tool)} />
            <span><b>{t(`tool.${tool}` as Key)}</b><div className="url">{tool}</div></span>
          </label>
        ))}
      </div>
      <h2>{t("ext.discovered")}</h2>
      {discovered.length === 0 ? <p className="hint">{t("ext.empty")}</p> : (
        <div className="cards">
          {discovered.map((record) => (
            <article key={`${record.kind}-${record.name}`} className="card">
              <h3>{record.name}</h3>
              <div className="url">{record.kind} · {record.detail}</div>
              <div className="url">{record.path}</div>
            </article>
          ))}
        </div>
      )}
      <button className="ghost" style={{ marginTop: 12 }} onClick={props.onOpenTerminal}>{t("ext.openTerminal")}</button>
    </div>
  );
}

function Shortcuts(props: { t: (key: Key) => string; lang: Lang }) {
  const column = props.lang === "zh-CN" ? 2 : 1;
  return (
    <div className="content">
      <div className="page-intro"><h1>{props.t("short.title")}</h1></div>
      <div className="shortcut-grid">
        <section className="panel">
          <h2>{props.t("short.desktop")}</h2>
          {desktopShortcuts.map((row) => (
            <div className="shortcut-row" key={row[0]}><kbd>{row[0]}</kbd><span>{row[column]}</span></div>
          ))}
        </section>
        <section className="panel">
          <h2>{props.t("short.tui")}</h2>
          {tuiShortcuts.map((row) => (
            <div className="shortcut-row" key={row[0]}><kbd>{row[0]}</kbd><span>{row[column]}</span></div>
          ))}
        </section>
      </div>
    </div>
  );
}

function ClassicTerminal(props: { t: (key: Key) => string; binary: string | null }) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);
  const shell = desktopShell();

  useEffect(() => {
    if (!host.current) return;
    const terminal = new Terminal({
      fontFamily: "ui-monospace, 'Sarasa Mono SC', 'Noto Sans Mono CJK SC', monospace",
      fontSize: 14,
      theme: { background: "#0b0d09", foreground: "#f3efe2", cursor: "#d6f15a" },
      cursorBlink: true,
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host.current);
    fit.fit();
    term.current = terminal;
    const onData = terminal.onData((data) => {
      const bytes = new TextEncoder().encode(data);
      let binaryText = "";
      bytes.forEach((byte) => { binaryText += String.fromCharCode(byte); });
      void writeTerminal(btoa(binaryText)).catch(() => undefined);
    });
    const observer = new ResizeObserver(() => {
      fit.fit();
      void resizeTerminal(terminal.cols, terminal.rows).catch(() => undefined);
    });
    observer.observe(host.current);
    return () => {
      observer.disconnect();
      onData.dispose();
      terminal.dispose();
      term.current = null;
    };
  }, []);

  async function launch() {
    if (!term.current) return;
    setLocalError(null);
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const binary = await startTerminal(term.current.cols, term.current.rows);
      setRunning(binary);
      const unlistenOut = await listen<string>("pty-output", (event) => {
        const raw = atob(event.payload);
        const bytes = Uint8Array.from(raw, (char) => char.charCodeAt(0));
        term.current?.write(bytes);
      });
      let unlistenExit = () => {};
      unlistenExit = await listen("pty-exit", () => {
        setRunning(null);
        unlistenOut();
        unlistenExit();
      });
    } catch (reason) {
      setLocalError(String(reason));
    }
  }

  const note = useMemo(() => localError ?? (!shell ? props.t("term.missing") : null), [localError, shell, props]);

  return (
    <div className="content" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div className="row">
        <div className="grow">
          <strong>{props.t("term.title")}</strong>
          <div className="hint">{props.t("term.body")}</div>
          {running && <div className="hint">{props.t("term.running")} {running}</div>}
          {note && <div className="hint">{note}</div>}
          {props.binary && <div className="hint">{props.binary}</div>}
        </div>
        {!running ? (
          <button className="primary" onClick={() => void launch()} disabled={!shell}>{props.t("term.start")}</button>
        ) : (
          <button className="danger" onClick={() => { void stopTerminal(); setRunning(null); }}>{props.t("term.stop")}</button>
        )}
      </div>
      <div className="terminal-wrap" ref={host} />
    </div>
  );
}
