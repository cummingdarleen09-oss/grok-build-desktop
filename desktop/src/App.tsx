import { memo, useEffect, useMemo, useRef, useState } from "react";
import {
  type AgentEvent,
  type Automation,
  type ChatOptions,
  type ReasoningEffort,
  type Bootstrap,
  type DiagnosticsInfo,
  type ExtensionRecord,
  type GitInfo,
  type MediaFile,
  type Provider,
  type ProviderStore,
  type Session,
  type SessionSummary,
  type Settings,
  branchChat,
  cancelPrompt,
  chatOptions,
  formatWindow,
  modelName,
  checkReady,
  copyText,
  createChat,
  diagnosticsInfo,
  gitInfo,
  listChats,
  loadAutomations,
  loadBootstrap,
  logFrontend,
  onProvidersUpdated,
  openLogFolder,
  readLogs,
  openChat,
  openClassic,
  pickProjectFolder,
  projectMedia,
  providerModels,
  removeChat,
  renameChat,
  renderDiagram,
  resolveApproval,
  saveSettings,
  sendPrompt,
  setChatModel,
  setChatOptions,
  storeAutomations,
} from "./api";
import { branchBanner, desktopShortcuts, effortName, translate, type Key, type Lang } from "./i18n";
import { ProvidersPanel } from "./ProvidersPanel";

type Page = "chat" | "automations" | "kanban" | "media" | "customize" | "settings" | "shortcuts";
type Hist = { page: Page; sessionId: string | null; readOnly: boolean };
/** An error shown to the user. `settings` adds an Open Settings action for configuration problems. */
type Problem = { text: string; settings: boolean };
type SetError = (error: string | null, settings?: boolean) => void;
const SIDEBAR_PAGE = 40;

function describe(reason: unknown): string {
  if (reason instanceof Error) return reason.message || String(reason);
  if (typeof reason === "string") return reason;
  try {
    return JSON.stringify(reason) ?? String(reason);
  } catch {
    return String(reason);
  }
}
const TOOLS = ["read_file", "write_file", "search_replace", "grep", "list_dir", "run_terminal"] as const;

function computeNextRun(item: Automation, from: Date): string {
  if (item.frequency === "once" && item.last_run_at) return "";
  const [hh, mm] = item.time.split(":").map((part) => Number(part) || 0);
  for (let day = 0; day < 14; day += 1) {
    const candidate = new Date(from);
    candidate.setDate(from.getDate() + day);
    candidate.setHours(hh, mm, 0, 0);
    if (candidate.getTime() <= from.getTime()) continue;
    const weekday = candidate.getDay();
    if (item.frequency === "weekdays" && (weekday === 0 || weekday === 6)) continue;
    if (item.frequency === "weekly" && item.weekdays.length > 0 && !item.weekdays.includes(weekday)) continue;
    return candidate.toISOString();
  }
  return "";
}

export function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [page, setPage] = useState<Page>("chat");
  const [session, setSession] = useState<Session | null>(null);
  const [readOnly, setReadOnly] = useState(false);
  const [focusMessageId, setFocusMessageId] = useState<string | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [bootFailed, setBootFailed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [plan, setPlan] = useState(false);
  const [sidebar, setSidebar] = useState(true);
  const [panel, setPanel] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const [showFilter, setShowFilter] = useState(false);
  const [menu, setMenu] = useState<string | null>(null);
  const [more, setMore] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [shown, setShown] = useState<Record<string, number>>({});
  const [git, setGit] = useState<GitInfo>({ current: "main", branches: ["main"] });
  const [branchName, setBranchName] = useState("main");
  const [picked, setPicked] = useState<{ providerId: string; model: string } | null>(null);
  const [waiting, setWaiting] = useState(false);
  const [canBack, setCanBack] = useState(false);
  const [canForward, setCanForward] = useState(false);
  const bootRef = useRef<Bootstrap | null>(null);
  const sessionRef = useRef<Session | null>(null);
  const historyRef = useRef<Hist[]>([{ page: "chat", sessionId: null, readOnly: false }]);
  const histIndex = useRef(0);
  const applyingHist = useRef(false);
  bootRef.current = boot;
  sessionRef.current = session;

  const setError: SetError = (text, settings = false) => {
    if (text) logFrontend("warn", text);
    setProblem(text ? { text, settings } : null);
  };
  const lang: Lang = boot?.settings.lang === "en" ? "en" : "zh-CN";
  const t = (key: Key) => translate(lang, key);
  const theme = boot?.settings.theme === "dark" ? "dark" : "light";

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  async function start() {
    setBootFailed(null);
    const started = performance.now();
    try {
      const next = await loadBootstrap();
      setBoot(next);
      logFrontend("info", `bootstrap rendered in ${Math.round(performance.now() - started)} ms`);
    } catch (reason) {
      const text = describe(reason);
      logFrontend("error", `bootstrap failed: ${text}`);
      setBootFailed(text);
    }
  }

  /** Re-read the chat list only. The full bootstrap also rescans extensions and providers. */
  async function refreshChats(openId?: string) {
    const sessions = await listChats();
    setBoot((current) => current ? { ...current, sessions } : current);
    if (openId && sessions.some((item) => item.id === openId && item.origin !== "classic")) {
      setReadOnly(false);
      setSession(await openChat(openId));
    }
  }

  useEffect(() => {
    void start();
    const onRejection = (event: PromiseRejectionEvent) => {
      const text = describe(event.reason);
      logFrontend("error", `unhandled rejection: ${text}`);
      setProblem({ text, settings: false });
    };
    const onError = (event: ErrorEvent) => {
      logFrontend("error", `uncaught: ${event.message} at ${event.filename}:${event.lineno}`);
      setProblem({ text: event.message || "error", settings: false });
    };
    window.addEventListener("unhandledrejection", onRejection);
    window.addEventListener("error", onError);
    let stop: (() => void) | undefined;
    let closed = false;
    void onProvidersUpdated((update) => {
      setBoot((current) => current ? { ...current, providers: update.providers, secrets: update.secrets } : current);
    }).then((unlisten) => { if (closed) unlisten(); else stop = unlisten; }).catch(() => undefined);
    return () => {
      closed = true;
      stop?.();
      window.removeEventListener("unhandledrejection", onRejection);
      window.removeEventListener("error", onError);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!boot) return;
    gitInfo(boot.settings.cwd).then((info) => {
      setGit(info);
      setBranchName(info.current);
    }).catch(() => undefined);
  }, [boot?.settings.cwd]);

  useEffect(() => {
    if (!boot || applyingHist.current) return;
    const entry = { page, sessionId: session?.id ?? null, readOnly };
    const current = historyRef.current[histIndex.current];
    if (current && current.page === entry.page && current.sessionId === entry.sessionId && current.readOnly === entry.readOnly) return;
    historyRef.current = historyRef.current.slice(0, histIndex.current + 1);
    historyRef.current.push(entry);
    histIndex.current = historyRef.current.length - 1;
    setCanBack(histIndex.current > 0);
    setCanForward(histIndex.current < historyRef.current.length - 1);
  }, [boot, page, session?.id, readOnly]);

  useEffect(() => {
    let stopped = false;
    async function tick() {
      const current = bootRef.current;
      if (!current || stopped) return;
      const now = new Date();
      const list = await loadAutomations();
      const prepared = list.map((item) => item.next_run_at ? item : { ...item, next_run_at: computeNextRun(item, now) });
      const due = prepared.filter((item) => item.enabled && item.prompt.trim() && item.next_run_at && new Date(item.next_run_at).getTime() <= now.getTime());
      const stamped = prepared.map((item) => {
        if (!due.some((entry) => entry.id === item.id)) return item;
        const once = item.frequency === "once";
        return {
          ...item,
          last_run_at: now.toISOString(),
          enabled: once ? false : item.enabled,
          next_run_at: once ? "" : computeNextRun(item, now),
        };
      });
      if (JSON.stringify(stamped) !== JSON.stringify(list)) {
        const saved = await storeAutomations(stamped);
        if (!stopped) setBoot((value) => value ? { ...value, automations: saved } : value);
      }
      for (const item of due) {
        if (stopped) return;
        await launchAutomation(item, false).catch((reason: unknown) => setError(`${item.title}: ${describe(reason)}`));
      }
    }
    if (!boot) return;
    const run = () => void tick().catch((reason: unknown) => logFrontend("warn", `automations: ${describe(reason)}`));
    const timer = window.setInterval(run, 30000);
    run();
    return () => { stopped = true; window.clearInterval(timer); };
  }, [boot == null]);

  const groups = useMemo(() => (boot ? projectGroups(boot, filter) : []), [boot, filter]);
  const flatChats = groups.flatMap((group) => (expanded[group.path] === false ? [] : group.sessions));

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (event.key === "Escape") {
        setSearchOpen(false);
        setMenu(null);
        setMore(false);
        if (busy) void cancelPrompt();
      }
      if (mod && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen(true);
      } else if (mod && event.key.toLowerCase() === "n") {
        event.preventDefault();
        void newChat();
      } else if (mod && event.key === ",") {
        event.preventDefault();
        setPage("settings");
      } else if (mod && event.key === "\\") {
        event.preventDefault();
        setSidebar((value) => !value);
      } else if (event.shiftKey && event.key === "Tab" && page === "chat") {
        event.preventDefault();
        setPlan((value) => !value);
      } else if (mod && /^[1-9]$/.test(event.key)) {
        const item = flatChats[Number(event.key) - 1];
        if (item) {
          event.preventDefault();
          void openListed(item);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  async function patchSettings(patch: Partial<Settings>) {
    if (!boot) return;
    const settings = { ...boot.settings, ...patch };
    await saveSettings(settings);
    setBoot({ ...boot, settings });
  }

  async function newChat() {
    if (!boot) return;
    setPage("chat");
    setReadOnly(false);
    setSession(null);
    setMore(false);
  }

  /** The chat to send into, or null after showing why nothing can be sent. */
  async function ensureChat(): Promise<Session | null> {
    const current = bootRef.current;
    if (!current) return null;
    const existing = session && !readOnly ? session : null;
    const chosen = current.providers.providers.find((item) => item.id === picked?.providerId && item.enabled);
    const provider = existing ? undefined : chosen ?? activeProvider(current);
    if (!existing && !provider) {
      setError(t("chat.noProvider"), true);
      return null;
    }
    try {
      await checkReady(existing ? existing.provider_id : provider!.id);
    } catch (reason) {
      setError(describe(reason), true);
      return null;
    }
    if (existing) return existing;
    const model = picked && picked.providerId === provider!.id ? picked.model : provider!.model;
    const created = await createChat(current.settings.cwd, provider!.id, model, lang);
    setSession(created);
    setReadOnly(false);
    void refreshChats().catch(() => undefined);
    return created;
  }

  async function addProject() {
    if (!boot) return;
    const picked = await pickProjectFolder();
    if (!picked) return;
    const path = projectPath(picked);
    const known = boot.settings.known_projects.some((item) => projectPath(item) === path)
      ? boot.settings.known_projects
      : [...boot.settings.known_projects, path];
    await patchSettings({ cwd: path, known_projects: known });
    setSession(null);
    setReadOnly(false);
    setPage("chat");
  }

  async function goHistory(delta: number) {
    const index = histIndex.current + delta;
    const entry = historyRef.current[index];
    if (!entry) return;
    applyingHist.current = true;
    histIndex.current = index;
    setCanBack(index > 0);
    setCanForward(index < historyRef.current.length - 1);
    setPage(entry.page);
    setReadOnly(entry.readOnly);
    try {
      if (!entry.sessionId) setSession(null);
      else setSession(entry.readOnly ? await openClassic(entry.sessionId) : await openChat(entry.sessionId));
    } catch (reason) {
      setError(describe(reason));
    } finally {
      applyingHist.current = false;
    }
  }

  async function launchAutomation(item: Automation, focus: boolean) {
    const current = bootRef.current;
    if (!current) return;
    const cwd = item.cwd || current.settings.cwd;
    const providerId = item.provider_id || current.settings.active_provider || "xai";
    const provider = current.providers.providers.find((entry) => entry.id === providerId) ?? activeProvider(current);
    if (!provider) {
      setError(t("chat.noProvider"), true);
      return;
    }
    try {
      await checkReady(provider.id);
    } catch (reason) {
      setError(`${item.title}: ${describe(reason)}`, true);
      return;
    }
    if (focus) await patchSettings({ cwd, active_provider: provider.id });
    const created = await createChat(cwd, provider.id, item.model || provider.model, current.settings.lang === "en" ? "en" : "zh-CN");
    if (focus) {
      setPage("chat");
      setReadOnly(false);
      setSession(created);
      setBusy(true);
    }
    try {
              const next = await sendPrompt(created.id, item.prompt, () => undefined, false, created.model);
      if (focus) setSession(next);
      else setNotice(item.title);
      await refreshChats();
    } finally {
      if (focus) setBusy(false);
    }
  }

  async function selectProject(path: string) {
    await patchSettings({ cwd: path });
    setExpanded((current) => ({ ...current, [path]: true }));
    setPage("chat");
  }

  async function openListed(item: SessionSummary) {
    if (!boot) return;
    await patchSettings({ cwd: projectPath(item.cwd) });
    setPage("chat");
    setFocusMessageId(null);
    setMore(false);
    if (item.origin === "classic") {
      setReadOnly(true);
      setSession(await openClassic(item.id));
      return;
    }
    setReadOnly(false);
    setSession(await openChat(item.id));
  }

  if (!boot) {
    const zh = !navigator.language.toLowerCase().startsWith("en");
    const say = (key: Key) => translate(zh ? "zh-CN" : "en", key);
    return (
      <div className="page boot-state">
        {bootFailed ? <>
          <h1>{say("boot.failed")}</h1>
          <pre className="boot-error">{bootFailed}</pre>
          <div className="row">
            <button className="primary" onClick={() => void start()}>{say("boot.retry")}</button>
            <button className="ghost" onClick={() => void readLogs().then(copyText).catch(() => undefined)}>{say("diag.copy")}</button>
            <button className="ghost" onClick={() => void openLogFolder().catch(() => undefined)}>{say("diag.open")}</button>
          </div>
        </> : <p className="hint">{say("boot.loading")}</p>}
      </div>
    );
  }

  return (
    <div className="app" onClick={() => { setMenu(null); }}>
      <header className="menubar">
        <div className="mark">G</div>
        {(["file", "edit", "view", "help"] as const).map((name) => (
          <div className="menu" key={name} onClick={(event) => event.stopPropagation()}>
            <button onClick={() => setMenu(menu === name ? null : name)}>{t(`menu.${name}` as Key)}</button>
            {menu === name && (
              <div className="menu-pop">
                {name === "file" && <>
                  <button onClick={() => { setMenu(null); void newChat(); }}>{t("menu.new")}</button>
                  <button onClick={() => { setMenu(null); void addProject(); }}>{t("menu.add")}</button>
                  <button onClick={() => { setMenu(null); setPage("settings"); }}>{t("menu.settings")}</button>
                </>}
                {name === "edit" && <button onClick={() => { setMenu(null); setSearchOpen(true); }}>{t("menu.search")}</button>}
                {name === "view" && <>
                  <button onClick={() => { setMenu(null); setSidebar((value) => !value); }}>{t("menu.sidebar")}</button>
                  <button onClick={() => { setMenu(null); setPanel((value) => !value); }}>{t("menu.panel")}</button>
                  <button onClick={() => { setMenu(null); void patchSettings({ theme: theme === "dark" ? "light" : "dark" }); }}>{theme === "dark" ? t("menu.light") : t("menu.dark")}</button>
                  <button onClick={() => { setMenu(null); void patchSettings({ lang: lang === "en" ? "zh-CN" : "en" }); }}>{t("menu.lang")}</button>
                </>}
                {name === "help" && <button onClick={() => { setMenu(null); setPage("shortcuts"); }}>{t("menu.shortcuts")}</button>}
              </div>
            )}
          </div>
        ))}
      </header>
      <div className={`workspace${sidebar ? "" : " side-closed"}${panel ? " with-panel" : ""}`}>
        <aside className="sidebar">
          <div className="side-tools">
            <button className="chrome-btn" title={t("menu.sidebar")} onClick={() => setSidebar(false)}>☰</button>
            <button className="chrome-btn" disabled={!canBack} onClick={() => void goHistory(-1)}>←</button>
            <button className="chrome-btn" disabled={!canForward} onClick={() => void goHistory(1)}>→</button>
          </div>
          <nav className="nav">
            <button className={`nav-item${page === "chat" && !session ? " active" : ""}`} onClick={() => void newChat()}>
              <Icon d="M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" />
              <span className="label">{t("nav.new")}</span>
              <kbd className="kbd">Ctrl+N</kbd>
            </button>
            <button className="nav-item" onClick={() => setSearchOpen(true)}>
              <Icon d="M11 19a8 8 0 1 1 0-16 8 8 0 0 1 0 16Zm10 2-5.2-5.2" />
              <span className="label">{t("nav.search")}</span>
              <kbd className="kbd">Ctrl+K</kbd>
            </button>
            <Nav page="automations" current={page} setPage={setPage} label={t("nav.automations")} icon="M12 8v4l3 2M12 22a10 10 0 1 1 0-20 10 10 0 0 1 0 20Z" />
            <Nav page="customize" current={page} setPage={setPage} label={t("nav.customize")} icon="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" />
          </nav>
          <div className="repo-head">
            <span>{t("repo.header")}</span>
            <button className="chrome-btn" title={t("repo.filter")} onClick={() => setShowFilter((value) => !value)}>⌕</button>
            <button className="chrome-btn" title={t("repo.add")} onClick={() => void addProject()}>＋</button>
          </div>
          {showFilter && <div className="repo-filter"><input value={filter} placeholder={t("repo.filter")} onChange={(event) => setFilter(event.target.value)} /></div>}
          <div className="repos">
            {groups.map((group) => {
              const open = expanded[group.path] !== false;
              let cursor = 0;
              for (const earlier of groups) {
                if (earlier.path === group.path) break;
                if (expanded[earlier.path] !== false) cursor += earlier.sessions.length;
              }
              return (
                <section key={group.path}>
                  <button className={projectPath(boot.settings.cwd) === group.path ? "repo-row active" : "repo-row"} onClick={() => { setExpanded((current) => ({ ...current, [group.path]: !open })); void selectProject(group.path); }}>
                    <Icon d={open ? "M3 7h6l2 2h10v10H3Z" : "M3 7h6l2 2h10v10H3Z"} />
                    <strong>{projectLeaf(group.path)}</strong>
                  </button>
                  {open && group.sessions.length === 0 && <div className="hint" style={{ paddingLeft: 28 }}>{t("repo.empty")}</div>}
                  {open && group.sessions.slice(0, shown[group.path] ?? SIDEBAR_PAGE).map((item, index) => (
                    <button key={`${item.origin}-${item.id}`} className={session?.id === item.id ? "chat-row active" : "chat-row"} onClick={() => void openListed(item).catch((reason: unknown) => setError(describe(reason)))}>
                      <span>{item.origin === "classic" ? "◌ " : item.parent_session_id ? "↳ " : ""}{item.title}</span>
                      {cursor + index < 9 && <kbd className="kbd">Ctrl+{cursor + index + 1}</kbd>}
                    </button>
                  ))}
                  {open && group.sessions.length > (shown[group.path] ?? SIDEBAR_PAGE) && (
                    <button className="chat-row more-row" onClick={() => setShown((current) => ({ ...current, [group.path]: (current[group.path] ?? SIDEBAR_PAGE) + SIDEBAR_PAGE * 4 }))}>
                      <span>{t("repo.more")} ({group.sessions.length - (shown[group.path] ?? SIDEBAR_PAGE)})</span>
                    </button>
                  )}
                </section>
              );
            })}
          </div>
          <div className="userbar">
            <div className="avatar">{boot.user_name.slice(0, 1).toUpperCase()}</div>
            <span className="grow">{boot.user_name}</span>
            <button className="chrome-btn" title={t("nav.settings")} onClick={() => setPage("settings")}>⚙</button>
          </div>
        </aside>
        <section className="stage">
          <div className="stage-tools">
            {!sidebar && <button onClick={() => setSidebar(true)}>{t("menu.sidebar")}</button>}
            <button onClick={() => setNotice(boot.settings.cwd)}>{t("chat.ide")} ↗</button>
            <button onClick={(event) => { event.stopPropagation(); setMore((value) => !value); }}>⋯</button>
            <button onClick={() => setPanel((value) => !value)}>⊟</button>
            {more && (
              <div className="pop" style={{ right: 0, left: "auto", top: 28 }} onClick={(event) => event.stopPropagation()}>
                <button onClick={() => { setMore(false); setPage("kanban"); }}>{t("nav.kanban")}</button>
                <button onClick={() => { setMore(false); setPage("media"); }}>{t("nav.media")}</button>
                <button onClick={() => { setMore(false); setPage("settings"); }}>{t("nav.settings")}</button>
                {session && !readOnly && <button onClick={() => { const title = window.prompt(t("chat.rename"), session.title); setMore(false); if (title) void renameChat(session.id, title).then((next) => { setSession(next); return refreshChats(); }).catch((reason: unknown) => setError(describe(reason))); }}>{t("chat.rename")}</button>}
                {session && !readOnly && <button onClick={() => { setMore(false); void removeChat(session.id).then(() => { setSession(null); return refreshChats(); }).catch((reason: unknown) => setError(describe(reason))); }}>{t("chat.delete")}</button>}
              </div>
            )}
          </div>
          {problem && page !== "chat" && <ErrorNote t={t} problem={problem} onDismiss={() => setError(null)} onSettings={() => { setError(null); setPage("settings"); }} />}
          {notice && <div className="banner" onClick={() => setNotice(null)}>{notice}</div>}
          {waiting && page !== "chat" && <button className="banner banner-action" onClick={() => setPage("chat")}>{t("chat.approvalWaiting")}</button>}
          <div className={page === "chat" ? "chat-host" : "chat-host is-hidden"}>
            <ChatStage
              t={t}
              lang={lang}
              boot={boot}
              session={session}
              readOnly={readOnly}
              busy={busy}
              plan={plan}
              setPlan={setPlan}
              git={git}
              branchName={branchName}
              setBranchName={setBranchName}
              focusMessageId={focusMessageId}
              setBusy={setBusy}
              problem={problem}
              setError={setError}
              setNotice={setNotice}
              setWaiting={setWaiting}
              picked={picked}
              onSettings={() => { setError(null); setPage("settings"); }}
              onEnsure={ensureChat}
              onOpen={(id, messageId) => void openListed(boot.sessions.find((item) => item.id === id) ?? { id, title: id, model: "", provider_id: "", cwd: boot.settings.cwd, updated_at: "", origin: "desktop" }).then(() => messageId && setFocusMessageId(messageId))}
              onBranch={async (messageId, includeMessage, replacement) => {
                if (!session) return;
                const created = await branchChat(session.id, messageId, includeMessage, lang);
                setSession(created);
                setReadOnly(false);
                if (replacement?.trim()) {
                  setBusy(true);
                  try {
                    const next = await sendPrompt(created.id, replacement.trim(), () => undefined, plan, created.model);
                    setSession(next);
                  } catch (reason) {
                    setError(describe(reason));
                    setSession(await openChat(created.id));
                  } finally { setBusy(false); }
                }
                await refreshChats();
              }}
              onSession={async (next) => { setSession(next); await refreshChats(); }}
              onProvider={async (providerId, model) => {
                setPicked({ providerId, model });
                try {
                  await patchSettings({ active_provider: providerId });
                  if (session && !readOnly) {
                    setSession(await setChatModel(session.id, providerId, model));
                  }
                } catch (reason) {
                  setError(describe(reason));
                }
              }}
            />
          </div>
          {page === "settings" && <SettingsView t={t} boot={boot} onSaved={(settings, providers) => setBoot({ ...boot, settings, providers })} setError={setError} />}
          {page === "customize" && <Customize t={t} records={boot.extensions} settings={boot.settings} onSettings={(settings) => setBoot({ ...boot, settings })} setError={setError} />}
          {page === "automations" && <Automations t={t} boot={boot} onRun={(item) => launchAutomation(item, true)} />}
          {page === "kanban" && <Kanban t={t} boot={boot} busyId={busy ? session?.id ?? null : null} onOpen={(item) => void openListed(item)} />}
          {page === "media" && <Media t={t} cwd={boot.settings.cwd} />}
          {page === "shortcuts" && <Shortcuts t={t} lang={lang} />}
        </section>
        {panel && (
          <aside className="aside">
            <h2 style={{ fontSize: 13 }}>{t("panel.session")}</h2>
            {session ? <>
              <p>{session.title}</p>
              <p className="hint">{session.model}</p>
              {session.parent_session_id && <p className="hint">{branchBanner(lang, parentTitle(boot, session.parent_session_id), session.branched_at_index ?? 0)}</p>}
            </> : <p className="hint">{t("panel.none")}</p>}
          </aside>
        )}
      </div>
      {searchOpen && <SearchDialog t={t} sessions={boot.sessions} onClose={() => setSearchOpen(false)} onOpen={(item) => { setSearchOpen(false); void openListed(item); }} />}
    </div>
  );
}

function Nav(props: { page: Page; current: Page; setPage: (page: Page) => void; label: string; icon: string }) {
  return (
    <button className={props.current === props.page ? "nav-item active" : "nav-item"} onClick={() => props.setPage(props.page)}>
      <Icon d={props.icon} />
      <span className="label">{props.label}</span>
    </button>
  );
}

function Icon(props: { d: string }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden>
      <path d={props.d} />
    </svg>
  );
}

function ErrorNote(props: { t: (key: Key) => string; problem: Problem; onDismiss: () => void; onSettings: () => void }) {
  return (
    <div className="error-note" role="alert">
      <span className="error-note__text">{props.problem.text}</span>
      <span className="error-note__actions">
        {props.problem.settings && <button type="button" className="primary" onClick={props.onSettings}>{props.t("chat.openSettings")}</button>}
        <button type="button" className="ghost" aria-label={props.t("chat.dismiss")} title={props.t("chat.dismiss")} onClick={props.onDismiss}>✕</button>
      </span>
    </div>
  );
}

function ChatStage(props: {
  t: (key: Key) => string;
  lang: Lang;
  boot: Bootstrap;
  session: Session | null;
  readOnly: boolean;
  busy: boolean;
  plan: boolean;
  setPlan: (value: boolean | ((value: boolean) => boolean)) => void;
  git: GitInfo;
  branchName: string;
  setBranchName: (value: string) => void;
  focusMessageId: string | null;
  setBusy: (busy: boolean) => void;
  problem: Problem | null;
  setError: SetError;
  setNotice: (notice: string | null) => void;
  setWaiting: (waiting: boolean) => void;
  picked: { providerId: string; model: string } | null;
  onSettings: () => void;
  onEnsure: () => Promise<Session | null>;
  onOpen: (id: string, messageId?: string | null) => void;
  onBranch: (messageId: string, includeMessage: boolean, replacement?: string) => Promise<void>;
  onSession: (session: Session) => Promise<void>;
  onProvider: (providerId: string, model: string) => Promise<void>;
}) {
  const { t, session } = props;
  const [draft, setDraft] = useState("");
  const [queue, setQueue] = useState<string[]>([]);
  const busyRef = useRef(false);
  const queueRef = useRef<string[]>([]);
  const [live, setLive] = useState("");
  const [reasoning, setReasoning] = useState("");
  const [status, setStatus] = useState<string[]>([]);
  const [pending, setPending] = useState<string | null>(null);
  const [tools, setTools] = useState<Session["messages"][number]["tool_calls"]>([]);
  const [approval, setApproval] = useState<{ id: string; name: string; arguments: string } | null>(null);
  const [allowed, setAllowed] = useState<string[]>([]);
  const allowedRef = useRef<string[]>([]);
  allowedRef.current = allowed;
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const scroller = useRef<HTMLDivElement>(null);
  const provider = props.session
    ? props.boot.providers.providers.find((item) => item.id === props.session?.provider_id) ?? activeProvider(props.boot)
    : activeProvider(props.boot);
  const dark = props.boot.settings.theme === "dark";
  const { setWaiting } = props;

  useEffect(() => {
    setWaiting(approval != null);
  }, [approval, setWaiting]);

  useEffect(() => {
    if (props.focusMessageId) {
      document.getElementById(`message-${props.focusMessageId}`)?.scrollIntoView({ block: "center" });
      return;
    }
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [session, live, tools, approval, status, props.focusMessageId]);

  function answer(id: string, approved: boolean) {
    setApproval(null);
    resolveApproval(id, approved).catch((reason: unknown) => props.setError(describe(reason)));
  }

  async function send(text: string) {
    const trimmed = text.trim();
    if (!trimmed) return;
    if (busyRef.current) {
      queueRef.current = [...queueRef.current, trimmed];
      setQueue(queueRef.current);
      setDraft("");
      return;
    }
    busyRef.current = true;
    props.setBusy(true);
    props.setError(null);
    let target: Session | null = null;
    try {
      target = await props.onEnsure();
      if (!target) {
        setDraft(trimmed);
        return;
      }
      setDraft("");
      setPending(trimmed);
      setLive("");
      setReasoning("");
      setStatus([]);
      setTools([]);
      const onEvent = (event: AgentEvent) => {
        if (event.type === "text_delta") setLive((value) => value + event.text);
        if (event.type === "reasoning_delta") setReasoning((value) => value + event.text);
        if (event.type === "status") setStatus((current) => [...current.filter((line) => line !== event.text), event.text]);
        if (event.type === "approval_needed") {
          if (allowedRef.current.includes(event.name)) void resolveApproval(event.id, true).catch(() => undefined);
          else setApproval({ id: event.id, name: event.name, arguments: event.arguments });
        }
        if (event.type === "tool_started") {
          setApproval(null);
          setTools((current) => [...current.filter((tool) => tool.id !== event.id), { id: event.id, name: event.name, arguments: event.arguments, output: null, is_error: false }]);
        }
        if (event.type === "tool_finished") {
          setTools((current) => current.map((tool) => tool.id === event.id ? { ...tool, output: event.output, is_error: event.is_error } : tool));
        }
        if (event.type === "error") props.setError(event.message);
      };
      const next = await sendPrompt(target.id, trimmed, onEvent, props.plan, target.model);
      setPending(null);
      setLive("");
      await props.onSession(next);
    } catch (reason) {
      setPending(null);
      setLive("");
      props.setError(describe(reason));
      if (target) {
        // The backend saves what it already has (the prompt, any partial reply).
        // Show that, and give the text back if the prompt never reached the chat.
        const failed = target;
        const reloaded = await openChat(failed.id).catch(() => null);
        if (reloaded) await props.onSession(reloaded).catch(() => undefined);
        const asked = reloaded?.messages.filter((message) => message.role === "user") ?? [];
        const last = asked[asked.length - 1];
        if (!last || last.text.trim() !== trimmed) setDraft((value) => value || trimmed);
      } else {
        setDraft((value) => value || trimmed);
      }
    } finally {
      setPending(null);
      setLive("");
      setReasoning("");
      setStatus([]);
      setTools([]);
      setApproval(null);
      busyRef.current = false;
      props.setBusy(false);
      const follow = queueRef.current[0];
      if (follow) {
        queueRef.current = queueRef.current.slice(1);
        setQueue(queueRef.current);
        void send(follow);
      }
    }
  }

  const branchesOf = useMemo(() => {
    const map = new Map<string, SessionSummary[]>();
    if (!session) return map;
    for (const item of props.boot.sessions) {
      if (item.parent_session_id !== session.id || !item.branched_from_message_id) continue;
      const list = map.get(item.branched_from_message_id) ?? [];
      list.push(item);
      map.set(item.branched_from_message_id, list);
    }
    return map;
  }, [props.boot.sessions, session]);

  const empty = !session || (session.messages.length === 0 && !props.busy);
  const note = props.problem && <ErrorNote t={t} problem={props.problem} onDismiss={() => props.setError(null)} onSettings={props.onSettings} />;
  const composer = (
    <Composer
      t={t}
      draft={draft}
      setDraft={setDraft}
      plan={props.plan}
      setPlan={props.setPlan}
      busy={props.busy}
      readOnly={props.readOnly}
      provider={provider}
      providers={props.boot.providers.providers}
      git={props.git}
      branchName={props.branchName}
      setBranchName={props.setBranchName}
      cwd={session?.cwd || props.boot.settings.cwd}
      model={session?.model || (props.picked && props.picked.providerId === provider?.id ? props.picked.model : provider?.model) || ""}
      lang={props.lang}
      sessionId={props.readOnly ? null : session?.id ?? null}
      setError={props.setError}
      onProvider={props.onProvider}
      onSend={() => void send(draft)}
      onAttach={() => {
        const path = window.prompt("@");
        if (path) setDraft((value) => `${value}${value ? " " : ""}@${path}`);
      }}
      onSlash={() => setDraft("/create-skill ")}
    />
  );

  // Streaming re-renders this component per token; the finished messages do not change meanwhile.
  const transcript = useMemo(() => session?.messages.map((message, index) => {
    const branches = branchesOf.get(message.id) ?? [];
    if (message.role === "compaction") {
      return (
        <details key={message.id} id={`message-${message.id}`} className="compaction">
          <summary>{t("chat.compacted")}</summary>
          <MessageBody text={message.text} dark={dark} />
        </details>
      );
    }
    return (
      <article key={message.id} id={`message-${message.id}`} className={`bubble ${message.role}`}>
        <div className="who">{message.role === "user" ? t("chat.you") : modelName(provider, session.model)} <span>#{index + 1}</span></div>
        {message.reasoning && <details className="reasoning"><summary>{t("chat.reasoning")}</summary><pre>{message.reasoning}</pre></details>}
        {editingId === message.id ? <textarea value={editText} onChange={(event) => setEditText(event.target.value)} /> : message.text && <MessageBody text={message.text} dark={dark} />}
        {message.tool_calls.map((tool) => <ToolCard key={tool.id} t={t} tool={tool} />)}
        {!props.readOnly && (
          <div className="msg-actions">
            <button type="button" className="icon-btn" title={t("chat.copy")} aria-label={t("chat.copy")} onClick={() => void copyText(message.text).catch(() => undefined)}>
              <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
            </button>
            <button type="button" className="icon-btn" title={t("chat.branch")} aria-label={t("chat.branch")} disabled={props.busy} onClick={() => void props.onBranch(message.id, true).catch((reason: unknown) => props.setError(describe(reason)))}>
              <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden><circle cx="6" cy="6" r="2.2" /><circle cx="6" cy="18" r="2.2" /><circle cx="18" cy="6" r="2.2" /><path d="M6 8.2v7.6M18 8.2a8 8 0 0 1-8 8" /></svg>
            </button>
            {message.role === "user" && editingId !== message.id && <button type="button" className="icon-btn" title={t("chat.edit")} aria-label={t("chat.edit")} disabled={props.busy} onClick={() => { setEditingId(message.id); setEditText(message.text); }}>{t("chat.edit")}</button>}
            {editingId === message.id && <button type="button" className="primary" disabled={props.busy || !editText.trim()} onClick={() => { const text = editText; setEditingId(null); void props.onBranch(message.id, false, text).catch((reason: unknown) => props.setError(describe(reason))); }}>{t("chat.resend")}</button>}
          </div>
        )}
        {branches.length > 0 && <div className="branch-list"><span>{t("chat.branches")}</span>{branches.map((item) => <button key={item.id} type="button" onClick={() => props.onOpen(item.id)}>{item.title}</button>)}</div>}
      </article>
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [session, branchesOf, editingId, editText, props.busy, props.readOnly, dark, props.lang, provider]);

  if (empty && !props.readOnly) {
    return (
      <div className="center-col">
        <div className="composer-wrap">
          {props.plan && <p className="hint">{t("chat.planOn")}</p>}
          {note}
          {composer}
          <div className="pills">
            <button className={props.plan ? "pill-btn on" : "pill-btn"} onClick={() => props.setPlan((value) => !value)}>{t("chat.plan")} ⇧Tab</button>
            <button className="pill-btn" onClick={() => void props.onEnsure().catch((reason: unknown) => props.setError(describe(reason)))}>{t("chat.multitask")}</button>
          </div>
        </div>
        <div className="suggests">
          {([
            ["chat.suggest1", "chat.suggest1b"],
            ["chat.suggest2", "chat.suggest2b"],
            ["chat.suggest3", "chat.suggest3b"],
            ["chat.suggest4", "chat.suggest4b"],
          ] as const).map(([title, body]) => (
            <button key={title} className="suggest" onClick={() => setDraft(t(title))}>
              <Icon d="M12 3v18M3 12h18" />
              <span><b>{t(title)}</b><em>{t(body)}</em></span>
            </button>
          ))}
        </div>
        <p className="footer-hint">{t("chat.footer")}</p>
      </div>
    );
  }

  return (
    <>
      <div className="transcript" ref={scroller}>
        {props.readOnly && <div className="branch-banner"><b>{t("chat.classic")}</b><div>{t("chat.classicBody")}</div></div>}
        {session?.parent_session_id && session.branched_at_index && (
          <button className="branch-banner" onClick={() => props.onOpen(session.parent_session_id || "", session.branched_from_message_id)}>
            {branchBanner(props.lang, parentTitle(props.boot, session.parent_session_id), session.branched_at_index)}
          </button>
        )}
        {transcript}
        {props.busy && pending && (
          <article className="bubble user pending">
            <div className="who">{t("chat.you")}</div>
            <MessageBody text={pending} dark={dark} streaming />
          </article>
        )}
        {props.busy && (
          <article className="bubble assistant">
            <div className="who">{t("chat.thinking")}</div>
            {!live && status.map((line) => <div key={line} className="hint status-line">{line}</div>)}
            {reasoning && <details className="reasoning" open><summary>{t("chat.reasoning")}</summary><pre>{reasoning}</pre></details>}
            {live && <MessageBody text={live} dark={dark} streaming />}
            {tools.map((tool) => <ToolCard key={tool.id} t={t} tool={tool} />)}
          </article>
        )}
      </div>
      <div className="dock">
        {approval && (
          <div className="approval">
            <div><strong>{t("chat.approval")}</strong><div className="meta">{approval.name}</div></div>
            <div className="row">
              <button className="primary" onClick={() => answer(approval.id, true)}>{t("chat.approve")}</button>
              <button onClick={() => { setAllowed((current) => [...current, approval.name]); answer(approval.id, true); }}>{t("chat.allowSession")}</button>
              <button className="danger" onClick={() => answer(approval.id, false)}>{t("chat.deny")}</button>
            </div>
          </div>
        )}
        {note}
        {queue.length > 0 && <div className="hint">{t("chat.queued")}: {queue.length}</div>}
        {props.plan && <p className="hint">{t("chat.planOn")}</p>}
        {composer}
      </div>
    </>
  );
}

function ModelMenu(props: {
  t: (key: Key) => string;
  lang: Lang;
  sessionId: string | null;
  providers: Provider[];
  providerId: string;
  model: string;
  setError: SetError;
  onPick: (providerId: string, model: string) => void;
}) {
  const { t } = props;
  const [open, setOpen] = useState(false);
  const [options, setOptions] = useState<ChatOptions | null>(null);
  const current = props.providers.find((item) => item.id === props.providerId);
  const groups = props.providers.filter((item) => item.enabled && providerModels(item).length > 0);
  const model = props.model || current?.model || "";

  useEffect(() => {
    if (!current || !model) {
      setOptions(null);
      return;
    }
    let live = true;
    chatOptions(props.sessionId, current.id, model)
      .then((value) => { if (live) setOptions(value); })
      .catch(() => { if (live) setOptions(null); });
    return () => { live = false; };
  }, [props.sessionId, current, model]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [open]);

  function choose(change: { effort?: ReasoningEffort; contextWindow?: number }) {
    if (!current) return;
    setChatOptions(props.sessionId, current.id, model, change)
      .then(setOptions)
      .catch((reason: unknown) => props.setError(describe(reason)));
  }

  const effort = options?.efforts.find((option) => option.value === options.effort);
  const parts = [current ? modelName(current, model) : model || "Grok 4.7"];
  if (effort) parts.push(effortName(props.lang, effort.value, effort.label));
  if (options) parts.push(formatWindow(options.context_window));
  const label = current ? `${current.name} · ${parts.join(" · ")}` : parts.join(" · ");
  const hasEffort = (options?.efforts.length ?? 0) > 0;
  const hasWindows = (options?.context_windows.length ?? 0) > 1;
  return (
    <div className="model-menu" onClick={(event) => event.stopPropagation()}>
      <button type="button" className="model-pick" aria-haspopup="listbox" aria-expanded={open} title={label} onClick={() => setOpen((value) => !value)}>
        {label} ▾
      </button>
      {open && (
        <div className={hasEffort || hasWindows ? "model-pop wide" : "model-pop"}>
          <div className="model-list" role="listbox" aria-label={t("prov.modelId")}>
            {groups.map((provider) => (
              <div key={provider.id} className="model-group">
                <div className="model-group__name">{provider.name}</div>
                {providerModels(provider).map((id) => {
                  const on = provider.id === props.providerId && id === model;
                  const name = modelName(provider, id);
                  return (
                    <button
                      key={`${provider.id}-${id}`}
                      type="button"
                      role="option"
                      aria-selected={on}
                      className={on ? "on" : ""}
                      title={id}
                      onClick={() => { if (!on) props.onPick(provider.id, id); }}
                    >
                      <span>{name}</span>
                      {name !== id && <span className="model-id">{id}</span>}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
          {options && (hasEffort || hasWindows) && (
            <div className="model-options">
              {hasEffort && (
                <div className="model-option" role="radiogroup" aria-label={t("model.effort")}>
                  <div className="model-group__name">{t("model.effort")}</div>
                  {options.efforts.map((option) => (
                    <button
                      key={option.id}
                      type="button"
                      role="radio"
                      aria-checked={option.value === options.effort}
                      className={option.value === options.effort ? "on" : ""}
                      title={option.description ?? undefined}
                      onClick={() => choose({ effort: option.value })}
                    >
                      {effortName(props.lang, option.value, option.label)}
                    </button>
                  ))}
                </div>
              )}
              <div className="model-option" role="radiogroup" aria-label={t("model.context")}>
                <div className="model-group__name">{t("model.context")}</div>
                {hasWindows ? options.context_windows.map((window) => (
                  <button
                    key={window}
                    type="button"
                    role="radio"
                    aria-checked={window === options.context_window}
                    className={window === options.context_window ? "on" : ""}
                    onClick={() => choose({ contextWindow: window })}
                  >
                    {formatWindow(window)}
                  </button>
                )) : <div className="model-static">{formatWindow(options.context_window)}</div>}
                <div className="hint">{t("model.compactAt").replace("{percent}", String(options.compact_at_percent))}</div>
              </div>
              <div className="hint">{t("model.defaultHint")}</div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Composer(props: {
  t: (key: Key) => string;
  draft: string;
  setDraft: (value: string) => void;
  plan: boolean;
  setPlan: (value: boolean | ((value: boolean) => boolean)) => void;
  busy: boolean;
  readOnly: boolean;
  provider: Provider | undefined;
  providers: Provider[];
  git: GitInfo;
  branchName: string;
  setBranchName: (value: string) => void;
  cwd: string;
  model: string;
  lang: Lang;
  sessionId: string | null;
  setError: SetError;
  onProvider: (id: string, model: string) => Promise<void>;
  onSend: () => void;
  onAttach: () => void;
  onSlash: () => void;
}) {
  const canSend = !props.readOnly && props.draft.trim().length > 0;
  return (
    <form onSubmit={(event) => { event.preventDefault(); if (canSend) props.onSend(); }}>
      <div className="context-row">
        <span className="chip">{projectLeaf(props.cwd)} ▾</span>
        <label className="chip">
          <select value={props.branchName} onChange={(event) => props.setBranchName(event.target.value)}>
            {props.git.branches.map((branch) => <option key={branch}>{branch}</option>)}
          </select>
        </label>
        <span className="chip">▣ {props.t("env.pc")} ▾</span>
      </div>
      <div className="composer-box">
        <textarea
          value={props.draft}
          disabled={props.readOnly}
          placeholder={props.t("chat.placeholder")}
          onChange={(event) => props.setDraft(event.target.value)}
          onKeyDown={(event) => {
            // Enter while an IME (Chinese, Japanese) is composing only commits the candidate.
            if (event.nativeEvent.isComposing || event.keyCode === 229) return;
            if (event.key === "Enter" && !event.shiftKey && !event.altKey) {
              event.preventDefault();
              if (canSend) props.onSend();
              return;
            }
            if (event.key === "/" && props.draft === "") {
              event.preventDefault();
              props.onSlash();
            }
          }}
        />
        <div className="composer-actions">
          <button type="button" className="plus" onClick={props.onAttach}>+</button>
          <ModelMenu
            t={props.t}
            lang={props.lang}
            sessionId={props.sessionId}
            providers={props.providers}
            providerId={props.provider?.id ?? ""}
            model={props.model || props.provider?.model || ""}
            setError={props.setError}
            onPick={(providerId, model) => void props.onProvider(providerId, model)}
          />
          <span className="send-hint">{props.t("chat.sendHint")}</span>
          {props.busy && <button type="button" className="danger" onClick={() => void cancelPrompt().catch(() => undefined)}>{props.t("chat.stop")}</button>}
          {(!props.busy || canSend) && (
            <button type="submit" className="send" disabled={!canSend} title={props.t("chat.send")} aria-label={props.t("chat.send")} data-testid="send">
              <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden><path d="M12 19V5M5 12l7-7 7 7" /></svg>
              <span>{props.t("chat.send")}</span>
            </button>
          )}
        </div>
      </div>
    </form>
  );
}

const MessageBody = memo(function MessageBody(props: { text: string; dark: boolean; streaming?: boolean }) {
  const parts = props.text.split(/```/);
  return (
    <div className="text">
      {parts.map((part, index) => {
        if (index % 2 === 0) return <span key={index}>{part}</span>;
        const nl = part.indexOf("\n");
        const lang = (nl === -1 ? part : part.slice(0, nl)).trim();
        const body = nl === -1 ? "" : part.slice(nl + 1);
        if (lang === "mermaid" && !props.streaming && index < parts.length - 1) return <Mermaid key={index} source={body} dark={props.dark} />;
        if (lang === "diff") return <pre key={index} className="diff">{body}</pre>;
        return <pre key={index}>{body || part}</pre>;
      })}
    </div>
  );
});

function Mermaid(props: { source: string; dark: boolean }) {
  const [svg, setSvg] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    renderDiagram(props.source, props.dark).then((value) => {
      if (!cancelled) setSvg(value ?? fallbackMermaid(props.source));
    }).catch(() => { if (!cancelled) setSvg(fallbackMermaid(props.source)); });
    return () => { cancelled = true; };
  }, [props.source, props.dark]);
  if (!svg) return <pre className="diagram">{props.source}</pre>;
  return <div className="diagram" dangerouslySetInnerHTML={{ __html: svg }} />;
}

function fallbackMermaid(source: string): string | null {
  const edges = source.split("\n").map((line) => line.trim()).filter((line) => line.includes("-->"));
  if (!edges.length) return null;
  const nodes = new Map<string, number>();
  const pairs: [string, string][] = [];
  for (const edge of edges) {
    const [left, right] = edge.split("-->").map((item) => item.trim());
    if (!left || !right) continue;
    if (!nodes.has(left)) nodes.set(left, nodes.size);
    if (!nodes.has(right)) nodes.set(right, nodes.size);
    pairs.push([left, right]);
  }
  const width = 220;
  const height = nodes.size * 64 + 24;
  const circles = [...nodes.entries()].map(([name, index]) => `<rect x="60" y="${20 + index * 64}" width="100" height="36" rx="8" fill="#fff" stroke="#d4d4d8"/><text x="110" y="${42 + index * 64}" text-anchor="middle" font-size="12" fill="#18181b">${name}</text>`);
  const lines = pairs.map(([left, right]) => {
    const a = nodes.get(left) ?? 0;
    const b = nodes.get(right) ?? 0;
    return `<line x1="110" y1="${56 + a * 64}" x2="110" y2="${20 + b * 64}" stroke="#71717a"/>`;
  });
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${lines.join("")}${circles.join("")}</svg>`;
}

function ToolCard(props: { t: (key: Key) => string; tool: Session["messages"][number]["tool_calls"][number] }) {
  const label = TOOLS.includes(props.tool.name as (typeof TOOLS)[number]) ? props.t(`tool.${props.tool.name}` as Key) : props.tool.name;
  let diff: { old: string; next: string } | null = null;
  if (props.tool.name === "search_replace") {
    try {
      const parsed = JSON.parse(props.tool.arguments) as { old?: string; new?: string };
      if (parsed.old && parsed.new) diff = { old: parsed.old, next: parsed.new };
    } catch { /* arguments may still be streaming */ }
  }
  return (
    <details className="tool" open={!props.tool.output}>
      <summary><b>{label}</b> <span className="hint">{props.tool.name}</span></summary>
      {diff ? <pre className="diff"><div className="del">- {diff.old}</div><div className="add">+ {diff.next}</div></pre> : <pre>{props.tool.arguments}</pre>}
      {props.tool.output && <pre>{props.tool.output}</pre>}
    </details>
  );
}

function SearchDialog(props: { t: (key: Key) => string; sessions: SessionSummary[]; onClose: () => void; onOpen: (item: SessionSummary) => void }) {
  const [query, setQuery] = useState("");
  const matches = props.sessions.filter((item) => `${item.title} ${item.cwd}`.toLowerCase().includes(query.toLowerCase())).slice(0, 12);
  return (
    <div className="search-back" onClick={props.onClose}>
      <div className="search-box" onClick={(event) => event.stopPropagation()}>
        <input autoFocus value={query} placeholder={props.t("search.placeholder")} onChange={(event) => setQuery(event.target.value)} />
        {matches.length === 0 && <div className="hint">{props.t("search.empty")}</div>}
        {matches.map((item) => <button key={item.id} onClick={() => props.onOpen(item)}>{item.title}<div className="hint">{projectLeaf(item.cwd)}</div></button>)}
      </div>
    </div>
  );
}

function Kanban(props: { t: (key: Key) => string; boot: Bootstrap; busyId: string | null; onOpen: (item: SessionSummary) => void }) {
  const columns: { key: Key; items: SessionSummary[] }[] = [
    { key: "kanban.input", items: props.boot.sessions.filter((item) => item.origin === "classic") },
    { key: "kanban.working", items: props.boot.sessions.filter((item) => item.id === props.busyId) },
    { key: "kanban.done", items: props.boot.sessions.filter((item) => item.origin !== "classic" && item.id !== props.busyId) },
  ];
  return (
    <div className="page">
      <h1>{props.t("nav.kanban")}</h1>
      <p className="lead">{props.t("kanban.body")}</p>
      <div className="kanban">
        {columns.map((column) => (
          <section key={column.key} className="panel">
            <h2>{props.t(column.key)}</h2>
            {column.items.length === 0 && <p className="hint">{props.t("kanban.empty")}</p>}
            {column.items.map((item) => <button key={item.id} onClick={() => props.onOpen(item)}>{item.title}<div className="hint">{projectLeaf(item.cwd)}</div></button>)}
          </section>
        ))}
      </div>
    </div>
  );
}

function Media(props: { t: (key: Key) => string; cwd: string }) {
  const [files, setFiles] = useState<MediaFile[]>([]);
  useEffect(() => { projectMedia(props.cwd).then(setFiles).catch(() => setFiles([])); }, [props.cwd]);
  return (
    <div className="page">
      <h1>{props.t("media.title")}</h1>
      <p className="lead">{props.t("media.body")}</p>
      {files.length === 0 ? <p className="hint">{props.t("media.empty")}</p> : (
        <div className="media-grid">
          {files.map((file) => (
            <article key={file.path} className="card">
              {file.kind === "image" && file.preview && <img src={file.preview} alt={file.name} />}
              {file.kind === "video" && <video src={file.preview || file.path} controls />}
              {file.kind === "audio" && <audio src={file.preview || file.path} controls />}
              <h3>{file.name}</h3>
              <div className="url">{file.kind}</div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}

function Automations(props: { t: (key: Key) => string; boot: Bootstrap; onRun: (item: Automation) => Promise<void> }) {
  const [list, setList] = useState<Automation[]>(props.boot.automations);
  useEffect(() => { loadAutomations().then(setList).catch(() => undefined); }, []);
  async function save(next: Automation[]) {
    setList(await storeAutomations(next));
  }
  return (
    <div className="page">
      <h1>{props.t("auto.title")}</h1>
      <p className="lead">{props.t("auto.body")}</p>
      <button className="primary" onClick={() => void save([...list, {
        id: crypto.randomUUID(),
        title: props.t("auto.add"),
        prompt: "",
        enabled: true,
        cwd: props.boot.settings.cwd,
        provider_id: props.boot.settings.active_provider ?? "xai",
        model: activeProvider(props.boot)?.model ?? "grok-4.7",
        frequency: "weekdays",
        time: "09:00",
        weekdays: [1, 2, 3, 4, 5],
        last_run_at: "",
        next_run_at: "",
      }])}>{props.t("auto.add")}</button>
      <div className="cards" style={{ marginTop: 12 }}>
        {list.map((item) => (
          <article key={item.id} className="card">
            <label className="field">{props.t("auto.titleField")}<input value={item.title} onChange={(event) => void save(list.map((entry) => entry.id === item.id ? { ...entry, title: event.target.value } : entry))} /></label>
            <label className="field">{props.t("auto.prompt")}<textarea value={item.prompt} onChange={(event) => void save(list.map((entry) => entry.id === item.id ? { ...entry, prompt: event.target.value } : entry))} /></label>
            <div className="row">
              <label className="field grow">{props.t("auto.frequency")}
                <select value={item.frequency} onChange={(event) => void save(list.map((entry) => entry.id === item.id ? { ...entry, frequency: event.target.value as Automation["frequency"] } : entry))}>
                  <option value="daily">{props.t("auto.daily")}</option>
                  <option value="weekdays">{props.t("auto.weekdays")}</option>
                  <option value="weekly">{props.t("auto.weekly")}</option>
                  <option value="once">{props.t("auto.once")}</option>
                </select>
              </label>
              <label className="field">{props.t("auto.time")}<input value={item.time} onChange={(event) => void save(list.map((entry) => entry.id === item.id ? { ...entry, time: event.target.value } : entry))} /></label>
            </div>
            <label className="checks"><input type="checkbox" checked={item.enabled} onChange={(event) => void save(list.map((entry) => entry.id === item.id ? { ...entry, enabled: event.target.checked } : entry))} />{props.t("auto.enabled")}</label>
            <button className="ghost" style={{ marginTop: 8 }} disabled={!item.prompt.trim()} onClick={() => void props.onRun(item)}>{props.t("auto.run")}</button>
          </article>
        ))}
      </div>
    </div>
  );
}

function Customize(props: {
  t: (key: Key) => string;
  records: ExtensionRecord[];
  settings: Settings;
  onSettings: (settings: Settings) => void;
  setError: (error: string | null) => void;
}) {
  const discovered = props.records.filter((record) => record.kind !== "tool");
  function toggle(tool: string) {
    const disabled = props.settings.disabled_tools.includes(tool)
      ? props.settings.disabled_tools.filter((item) => item !== tool)
      : [...props.settings.disabled_tools, tool];
    const settings = { ...props.settings, disabled_tools: disabled };
    void saveSettings(settings).then(() => props.onSettings(settings)).catch((reason: unknown) => props.setError(describe(reason)));
  }
  return (
    <div className="page">
      <h1>{props.t("customize.title")}</h1>
      <p className="lead">{props.t("customize.body")}</p>
      <h2>{props.t("customize.tools")}</h2>
      <div className="cards">
        {TOOLS.map((tool) => (
          <label key={tool} className="card checks">
            <input type="checkbox" checked={!props.settings.disabled_tools.includes(tool)} onChange={() => toggle(tool)} />
            <span><b>{props.t(`tool.${tool}` as Key)}</b><div className="url">{tool}</div></span>
          </label>
        ))}
      </div>
      <h2>{props.t("customize.found")}</h2>
      {discovered.length === 0 ? <p className="hint">{props.t("customize.empty")}</p> : (
        <div className="cards">
          {discovered.map((record) => (
            <article key={`${record.kind}-${record.name}`} className="card">
              <h3>{record.name}</h3>
              <div className="url">{record.kind} · {record.detail}</div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}

function Shortcuts(props: { t: (key: Key) => string; lang: Lang }) {
  const column = props.lang === "zh-CN" ? 2 : 1;
  return (
    <div className="page">
      <h1>{props.t("short.title")}</h1>
      <section className="panel">
        {desktopShortcuts.map((row) => <div className="row" key={row[0]}><kbd className="kbd">{row[0]}</kbd><span>{row[column]}</span></div>)}
      </section>
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
  const [notice, setNotice] = useState<string | null>(null);
  async function save() {
    props.setError(null);
    try {
      await saveSettings(settings);
      props.onSaved(settings, { active: settings.active_provider, providers });
      setNotice(t("settings.saved"));
    } catch (reason) {
      props.setError(String(reason));
    }
  }
  return (
    <div className="page">
      <h1>{t("nav.settings")}</h1>
      <p className="lead">{t("settings.providersHint")}</p>
      <div className="panel">
        <div className="row">
          <label className="field grow">{t("settings.cwd")}<input value={settings.cwd} onChange={(event) => setSettings({ ...settings, cwd: event.target.value })} /></label>
          <label className="field">{t("settings.permission")}
            <select value={settings.permission} onChange={(event) => setSettings({ ...settings, permission: event.target.value as Settings["permission"] })}>
              <option value="ask">{t("settings.ask")}</option>
              <option value="auto">{t("settings.auto")}</option>
            </select>
          </label>
          <label className="field">{t("settings.theme")}
            <select value={settings.theme} onChange={(event) => setSettings({ ...settings, theme: event.target.value as Settings["theme"] })}>
              <option value="light">{t("menu.light")}</option>
              <option value="dark">{t("menu.dark")}</option>
            </select>
          </label>
          <label className="field">{t("settings.language")}
            <select value={settings.lang} onChange={(event) => setSettings({ ...settings, lang: event.target.value as Lang })}>
              <option value="zh-CN">中文</option>
              <option value="en">English</option>
            </select>
          </label>
        </div>
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="primary" onClick={() => void save()}>{t("settings.save")}</button>
        {notice && <span className="hint">{notice}</span>}
      </div>
      <ProvidersPanel
        t={t}
        settings={settings}
        providers={providers}
        secrets={props.boot.secrets}
        setError={props.setError}
        onSaved={(nextSettings, store) => {
          setSettings(nextSettings);
          setProviders(store.providers);
          props.onSaved(nextSettings, store);
        }}
      />
      <Diagnostics t={t} setError={props.setError} />
    </div>
  );
}

function Diagnostics(props: { t: (key: Key) => string; setError: (error: string | null) => void }) {
  const { t } = props;
  const [info, setInfo] = useState<DiagnosticsInfo | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => { diagnosticsInfo().then(setInfo).catch(() => setInfo(null)); }, []);
  async function copy() {
    try {
      await copyText(await readLogs());
      setNotice(t("diag.copied"));
    } catch (reason) {
      props.setError(describe(reason));
    }
  }
  return (
    <section className="panel diagnostics" data-testid="diagnostics">
      <h2>{t("diag.title")}</h2>
      <p className="hint">{t("diag.body")}</p>
      {info && <p className="url">{t("diag.path")}: <code>{info.log_path}</code> · v{info.version} · {info.os} {info.arch}</p>}
      <div className="row">
        <button type="button" className="primary" onClick={() => void copy()}>{t("diag.copy")}</button>
        <button type="button" className="ghost" onClick={() => void openLogFolder().catch((reason: unknown) => props.setError(describe(reason)))}>{t("diag.open")}</button>
        {notice && <span className="hint">{notice}</span>}
      </div>
    </section>
  );
}

function projectPath(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, "");
  return trimmed || path.trim();
}
function projectLeaf(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}
function projectGroups(boot: Bootstrap, filter: string): { path: string; sessions: SessionSummary[] }[] {
  const paths = new Map<string, string>();
  const add = (path: string) => { const key = projectPath(path); if (key) paths.set(key, key); };
  add(boot.settings.cwd);
  for (const path of boot.settings.known_projects ?? []) add(path);
  for (const session of boot.sessions) add(session.cwd);
  const needle = filter.trim().toLowerCase();
  return [...paths.values()]
    .filter((path) => !needle || projectLeaf(path).toLowerCase().includes(needle) || path.toLowerCase().includes(needle))
    .sort((left, right) => projectLeaf(left).localeCompare(projectLeaf(right)))
    .map((path) => ({
      path,
      sessions: boot.sessions.filter((session) => projectPath(session.cwd) === path).sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    }));
}
function parentTitle(boot: Bootstrap, id: string): string {
  return boot.sessions.find((session) => session.id === id)?.title ?? id;
}
function activeProvider(boot: Bootstrap): Provider | undefined {
  return boot.providers.providers.find((item) => item.id === boot.settings.active_provider && item.enabled)
    ?? boot.providers.providers.find((item) => item.enabled);
}
