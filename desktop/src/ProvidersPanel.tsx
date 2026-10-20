import { useEffect, useState } from "react";
import {
  type GrokAccount,
  type ModelInfo,
  type Provider,
  type ProviderFormat,
  type ProviderStore,
  type SecretStatus,
  type Settings,
  clearSecret,
  desktopShell,
  fetchModels,
  formatWindow,
  grokAccount,
  grokLogin,
  grokLoginCancel,
  grokLogout,
  grokRefresh,
  modelName,
  openExternalUrl,
  providerModels,
  providerTemplates,
  saveProviders,
  setSecret,
  testProvider,
} from "./api";
import type { Key } from "./i18n";

type Mode = "empty" | "presets" | "official" | "edit";

interface FormState {
  id: string;
  name: string;
  format: ProviderFormat;
  base_url: string;
  apiKey: string;
  models: string[];
  enabled: boolean;
  existing: boolean;
  /** Per-model overrides as typed: windows like "200K, 1M", effort auto/on/off, Claude thinking style. */
  tuning: Record<string, ModelTuning>;
}

interface ModelTuning {
  windows: string;
  effort: "auto" | "on" | "off";
  thinking: "auto" | "adaptive" | "budget";
}

function blankForm(): FormState {
  return { id: "", name: "", format: "responses", base_url: "https://", apiKey: "", models: [], enabled: true, existing: false, tuning: {} };
}

function tuningOf(entry: ModelInfo | undefined): ModelTuning {
  const windows = entry?.context_window != null ? [entry.context_window, ...(entry.context_windows ?? []).filter((window) => window !== entry.context_window)] : entry?.context_windows ?? [];
  return {
    windows: windows.map(formatWindow).join(", "),
    effort: entry?.supports_reasoning_effort == null && !entry?.reasoning_efforts?.length ? "auto" : entry?.supports_reasoning_effort === false ? "off" : "on",
    thinking: entry?.thinking ?? "auto",
  };
}

function tuningFor(models: string[], entries: ModelInfo[] | undefined): Record<string, ModelTuning> {
  return Object.fromEntries(models.map((id) => [id, tuningOf(entries?.find((entry) => entry.id === id))]));
}

/** "200K, 1M" -> [200000, 1000000]; null when something is not a size. */
export function parseWindows(text: string): number[] | null {
  const parts = text.split(/[\s,，]+/).map((part) => part.trim()).filter(Boolean);
  const out: number[] = [];
  for (const part of parts) {
    const match = /^(\d+(?:\.\d+)?)([kKmM]?)$/.exec(part);
    if (!match) return null;
    const scale = match[2].toLowerCase() === "m" ? 1_000_000 : match[2].toLowerCase() === "k" ? 1000 : 1;
    const value = Math.round(Number(match[1]) * scale);
    if (value < 1000) return null;
    if (!out.includes(value)) out.push(value);
  }
  return out;
}

/** Fold the form's overrides into the saved entries, keeping server fields such as names and effort menus. */
function applyTuning(models: string[], tuning: Record<string, ModelTuning>, previous: ModelInfo[] | undefined): ModelInfo[] {
  const out: ModelInfo[] = [];
  for (const id of models) {
    const entry: ModelInfo = { ...(previous?.find((item) => item.id === id) ?? { id }) };
    const wanted = tuning[id];
    if (wanted) {
      const windows = parseWindows(wanted.windows) ?? [];
      delete entry.context_window;
      delete entry.context_windows;
      if (windows.length) {
        entry.context_window = windows[0];
        if (windows.length > 1) entry.context_windows = windows.slice(1);
      }
      if (wanted.effort === "auto") {
        delete entry.supports_reasoning_effort;
      } else if (wanted.effort === "off") {
        entry.supports_reasoning_effort = false;
        delete entry.reasoning_efforts;
        delete entry.reasoning_effort;
      } else {
        entry.supports_reasoning_effort = true;
      }
      if (wanted.thinking === "auto") delete entry.thinking;
      else entry.thinking = wanted.thinking;
    }
    if (Object.keys(entry).length > 1) out.push(entry);
  }
  return out;
}

function slugify(name: string): string {
  const slug = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return slug.slice(0, 40);
}

function uniqueId(base: string, taken: string[]): string {
  const root = slugify(base) || "provider";
  if (!taken.includes(root)) return root;
  let n = 2;
  while (taken.includes(`${root}-${n}`)) n += 1;
  return `${root}-${n}`;
}

function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** Most OpenAI-compatible relays (DeepSeek, Kimi, one-api) reject max_completion_tokens; only OpenAI itself requires it. */
function tokenField(format: ProviderFormat, baseUrl: string, previous?: Provider): Provider["token_field"] {
  if (format !== "openai") return "max_tokens";
  if (previous && previous.format === "openai") return previous.token_field;
  const host = hostOf(baseUrl).toLowerCase();
  return host.endsWith("openai.com") || host.endsWith("openai.azure.com") ? "max_completion_tokens" : "max_tokens";
}

function letter(name: string): string {
  return (name.trim().slice(0, 1) || "?").toUpperCase();
}

export function ProvidersPanel(props: {
  t: (key: Key) => string;
  settings: Settings;
  providers: Provider[];
  secrets: SecretStatus[];
  onSaved: (settings: Settings, providers: ProviderStore) => void;
  setError: (error: string | null) => void;
}) {
  const { t } = props;
  const [providers, setProviders] = useState(props.providers);
  const [secrets, setSecrets] = useState(props.secrets);
  const [active, setActive] = useState(props.settings.active_provider);
  const [mode, setMode] = useState<Mode>("official");
  const [selection, setSelection] = useState<string>("xai");
  const [form, setForm] = useState<FormState>(blankForm());
  const [showKey, setShowKey] = useState(false);
  const [draftModel, setDraftModel] = useState("");
  const [remote, setRemote] = useState<string[]>([]);
  const [fetching, setFetching] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const grok = providers.find((provider) => provider.id === "xai");
  const custom = providers.filter((provider) => provider.id !== "xai");
  const columns = form.format === "anthropic" ? "minmax(0, 1fr) 110px 130px 130px 28px" : "minmax(0, 1fr) 110px 130px 28px";

  async function persist(next: Provider[], nextActive: string | null) {
    const saved = await saveProviders({ active: nextActive, providers: next });
    setProviders(saved.providers);
    setActive(saved.active);
    props.onSaved({ ...props.settings, active_provider: saved.active }, saved);
    return saved;
  }

  function openOfficial() {
    setSelection("xai");
    setMode("official");
    setNotice(null);
    setRemote([]);
  }

  function openCreate() {
    setMode("presets");
    setSelection("");
    setNotice(null);
    setRemote([]);
  }

  function openCustom() {
    setForm(blankForm());
    setShowKey(false);
    setDraftModel("");
    setRemote([]);
    setMode("edit");
    setNotice(null);
  }

  function openTemplate(id: string) {
    const template = providerTemplates.find((item) => item.id === id);
    if (!template) return;
    const taken = providers.map((provider) => provider.id);
    setForm({
      id: uniqueId(template.id, taken),
      name: template.name,
      format: template.format,
      base_url: template.base_url,
      apiKey: "",
      models: [...template.models],
      enabled: true,
      existing: false,
      tuning: tuningFor(template.models, undefined),
    });
    setShowKey(false);
    setDraftModel("");
    setRemote([]);
    setMode("edit");
    setNotice(null);
  }

  function openEdit(provider: Provider) {
    setSelection(provider.id);
    setForm({
      id: provider.id,
      name: provider.name,
      format: provider.format,
      base_url: provider.base_url,
      apiKey: "",
      models: providerModels(provider),
      enabled: provider.enabled,
      existing: true,
      tuning: tuningFor(providerModels(provider), provider.model_info),
    });
    setShowKey(false);
    setDraftModel("");
    setRemote([]);
    setMode("edit");
    setNotice(null);
  }

  function addDraftModel() {
    const id = draftModel.trim();
    if (!id || form.models.includes(id)) return;
    setForm({ ...form, models: [...form.models, id], tuning: { ...form.tuning, [id]: form.tuning[id] ?? tuningOf(undefined) } });
    setDraftModel("");
  }

  async function onFetch() {
    setFetching(true);
    props.setError(null);
    try {
      const models = await fetchModels(form.id, form.base_url, form.format, form.apiKey);
      setRemote(models);
      setNotice(String(models.length));
    } catch (reason) {
      props.setError(String(reason));
    } finally {
      setFetching(false);
    }
  }

  async function onSave() {
    if (!form.name.trim()) {
      props.setError(t("prov.needName"));
      return;
    }
    if (!(form.base_url.startsWith("http://") || form.base_url.startsWith("https://"))) {
      props.setError(t("prov.needBase"));
      return;
    }
    const models = form.models.map((model) => model.trim()).filter(Boolean);
    if (models.length === 0) {
      props.setError(t("prov.needModel"));
      return;
    }
    const badWindow = models.find((id) => form.tuning[id] && parseWindows(form.tuning[id].windows) == null);
    if (badWindow) {
      props.setError(`${t("prov.badWindow")}: ${badWindow}`);
      return;
    }
    const hasKey = Boolean(form.apiKey.trim()) || secrets.some((secret) => secret.id === form.id && (secret.location || secret.from_env || secret.from_session));
    if (!form.existing && !hasKey) {
      props.setError(t("prov.needKey"));
      return;
    }
    const taken = providers.filter((provider) => provider.id !== form.id).map((provider) => provider.id);
    const id = form.existing ? form.id : uniqueId(form.id || form.name, taken);
    const previous = providers.find((provider) => provider.id === id);
    const envKey = previous?.env_key || `${id.replace(/-/g, "_").toUpperCase()}_API_KEY`;
    const anthropic = form.format === "anthropic";
    const provider: Provider = {
      id,
      name: form.name.trim(),
      format: form.format,
      base_url: form.base_url.trim().replace(/\/+$/, ""),
      model: models[0],
      env_key: envKey,
      env_key_fallbacks: previous?.env_key_fallbacks ?? [],
      context_window: previous?.context_window ?? (anthropic ? 200000 : 128000),
      extra_headers: previous && previous.format === form.format ? previous.extra_headers : anthropic ? { "anthropic-version": "2023-06-01" } : {},
      env_http_headers: anthropic ? { "x-api-key": envKey } : {},
      auth: anthropic ? "x_api_key" : "bearer",
      enabled: form.enabled,
      temperature: previous?.temperature ?? null,
      max_tokens: previous?.max_tokens ?? 8192,
      token_field: tokenField(form.format, form.base_url, previous),
      builtin: false,
      model_hints: models,
      models,
      follow_latest: false,
      model_info: applyTuning(models, form.tuning, previous?.model_info),
    };
    setBusy(true);
    props.setError(null);
    try {
      if (form.apiKey.trim()) {
        const location = await setSecret(id, form.apiKey.trim());
        setSecrets((current) => {
          const rest = current.filter((secret) => secret.id !== id);
          return [...rest, { id, location, from_env: false, from_session: false }];
        });
      }
      const next = [...providers.filter((item) => item.id !== id), provider];
      const saved = await persist(next, active);
      const stored = saved.providers.find((item) => item.id === id);
      setForm({ ...form, id, apiKey: "", existing: true, models, tuning: tuningFor(models, stored?.model_info) });
      setSelection(id);
      setNotice(t("prov.saved"));
    } catch (reason) {
      props.setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function onDelete() {
    if (!form.existing || form.id === "xai") return;
    if (!window.confirm(t("prov.deleteAsk"))) return;
    setBusy(true);
    try {
      await clearSecret(form.id).catch(() => undefined);
      setSecrets((current) => current.filter((secret) => secret.id !== form.id));
      const nextActive = active === form.id ? "xai" : active;
      await persist(providers.filter((provider) => provider.id !== form.id), nextActive);
      openOfficial();
    } catch (reason) {
      props.setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function onTest(id: string, model?: string) {
    setBusy(true);
    props.setError(null);
    try {
      setNotice(await testProvider(id, model));
    } catch (reason) {
      props.setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function useProvider(id: string) {
    const next = providers.map((provider) => provider.id === id ? { ...provider, enabled: true } : provider);
    try {
      await persist(next, id);
      setSelection(id);
    } catch (reason) {
      props.setError(String(reason));
    }
  }

  async function pinGrok(follow: boolean, model: string) {
    if (!grok) return;
    const next = providers.map((provider) => provider.id === "xai" ? { ...provider, follow_latest: follow, model } : provider);
    try {
      await persist(next, active);
    } catch (reason) {
      props.setError(String(reason));
    }
  }

  return (
    <section className="prov-panel" data-testid="providers-panel">
      <h2>{t("prov.title")}</h2>
      <div className="prov-split">
        <aside className="prov-split__list">
          <button type="button" className="primary prov-add" onClick={openCreate}>{t("prov.new")}</button>
          {grok && (
            <div className={`prov-item${selection === "xai" && mode === "official" ? " is-selected" : ""}${active === "xai" ? " is-active" : ""}`}>
              <button type="button" className="prov-item__main" onClick={openOfficial}>
                <span className="prov-item__avatar">G</span>
                <span className="prov-item__text">
                  <span className="prov-item__name">{t("prov.officialName")}</span>
                  <span className="prov-item__sub">{grok.model}</span>
                </span>
              </button>
              {active === "xai" ? <span className="prov-item__using" title={t("prov.active")}>✓</span> : <button type="button" className="ghost" onClick={() => void useProvider("xai")}>{t("prov.useThis")}</button>}
            </div>
          )}
          {custom.map((provider) => (
            <div key={provider.id} className={`prov-item${selection === provider.id && mode === "edit" ? " is-selected" : ""}${active === provider.id ? " is-active" : ""}${provider.enabled ? "" : " is-off"}`}>
              <button type="button" className="prov-item__main" onClick={() => openEdit(provider)}>
                <span className="prov-item__avatar">{letter(provider.name)}</span>
                <span className="prov-item__text">
                  <span className="prov-item__name">{provider.name}</span>
                  <span className="prov-item__sub">{hostOf(provider.base_url)} · {provider.model}{provider.enabled ? "" : ` · ${t("prov.disabled")}`}</span>
                </span>
              </button>
              {active === provider.id ? <span className="prov-item__using" title={t("prov.active")}>✓</span> : <button type="button" className="ghost" onClick={() => void useProvider(provider.id)}>{t("prov.useThis")}</button>}
            </div>
          ))}
        </aside>
        <div className="prov-split__detail">
          {mode === "empty" && <div className="prov-detail-empty">{t("prov.detailEmpty")}</div>}
          {mode === "presets" && (
            <div className="prov-detail" data-testid="provider-presets">
              <div className="prov-form__head">
                <h3>{t("prov.presetsTitle")}</h3>
                <button type="button" className="chrome-btn" onClick={openOfficial}>×</button>
              </div>
              <p className="hint">{t("prov.presetsHint")}</p>
              <div className="prov-presets">
                <button type="button" className="prov-presets__chip prov-presets__chip--custom" onClick={openCustom}>{t("prov.presetCustom")}</button>
                {providerTemplates.map((template) => (
                  <button key={template.id} type="button" className="prov-presets__chip" onClick={() => openTemplate(template.id)}>
                    <span className="prov-presets__avatar">{letter(template.name)}</span>
                    <span>{template.name}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
          {mode === "official" && grok && (
            <div className="prov-detail" data-testid="official-grok">
              <div className="prov-detail__head">
                <div>
                  <h3>{t("prov.officialName")}</h3>
                  <p className="hint">{t("prov.officialDesc")}</p>
                </div>
                {active === "xai" ? <span className="prov-badge">{t("prov.active")}</span> : <button type="button" className="primary" onClick={() => void useProvider("xai")}>{t("prov.useThis")}</button>}
              </div>
              <OfficialAccount t={t} setError={props.setError} />
              <label className="checks"><input type="checkbox" checked={grok.follow_latest} onChange={(event) => void pinGrok(event.target.checked, grok.model)} />{t("settings.latest")}</label>
              <p className="hint">{t("settings.latestHint")}</p>
              <label className="field">{t("settings.model")}
                <select value={grok.model} onChange={(event) => void pinGrok(false, event.target.value)}>
                  {providerModels(grok).map((model) => <option key={model} value={model}>{modelName(grok, model)}</option>)}
                </select>
              </label>
              <GrokKey t={t} onSave={async (secret) => {
                const location = await setSecret("xai", secret);
                setSecrets((current) => current.map((item) => item.id === "xai" ? { ...item, location } : item));
                setNotice(t("prov.saved"));
              }} setError={props.setError} />
              <div className="prov-form__actions">
                <button type="button" className="ghost" disabled={busy} onClick={() => void onTest("xai", grok.model)}>{t("prov.test")}</button>
              </div>
              {notice && <p className="hint">{notice}</p>}
            </div>
          )}
          {mode === "edit" && (
            <div className="prov-detail prov-form" data-testid="provider-form" role="dialog" aria-label={form.existing ? t("prov.editTitle") : t("prov.addTitle")}>
              <div className="prov-form__head">
                <h3>{form.existing ? t("prov.editTitle") : t("prov.addTitle")}</h3>
                <button type="button" className="chrome-btn" onClick={openOfficial}>×</button>
              </div>
              <div className="prov-form__grid">
                <label className="field">{t("prov.name")}
                  <input value={form.name} placeholder={t("prov.namePh")} onChange={(event) => {
                    const name = event.target.value;
                    setForm((current) => ({ ...current, name, id: current.existing ? current.id : slugify(name) }));
                  }} />
                </label>
                <label className="field">{t("settings.id")}
                  <input value={form.id} readOnly={form.existing} onChange={(event) => setForm({ ...form, id: slugify(event.target.value) })} />
                </label>
                <fieldset className="field prov-field--full">
                  <legend>{t("prov.protocol")}</legend>
                  <div className="prov-formats" role="radiogroup" aria-label={t("prov.protocol")}>
                    {(["anthropic", "openai", "responses"] as const).map((format) => (
                      <label key={format} className={form.format === format ? "on" : ""}>
                        <input type="radio" name="api-format" value={format} checked={form.format === format} onChange={() => setForm({ ...form, format })} />
                        {t(format === "anthropic" ? "prov.format.messages" : format === "openai" ? "prov.format.chat" : "prov.format.responses")}
                      </label>
                    ))}
                  </div>
                </fieldset>
                <label className="field prov-field--full">{t("prov.baseUrl")}
                  <input value={form.base_url} onChange={(event) => setForm({ ...form, base_url: event.target.value })} />
                </label>
                <label className="field prov-field--full">{t("prov.apiKey")}
                  <span className="prov-key-row">
                    <input type={showKey ? "text" : "password"} autoComplete="off" value={form.apiKey} placeholder={form.existing ? t("prov.keyKeep") : "sk-…"} onChange={(event) => setForm({ ...form, apiKey: event.target.value })} />
                    <button type="button" className="ghost" onClick={() => setShowKey((value) => !value)}>{showKey ? t("prov.keyHide") : t("prov.keyShow")}</button>
                  </span>
                </label>
                <div className="field prov-field--full">
                  <span>{t("prov.models")}</span>
                  <div className="prov-models">
                    {form.models.length === 0 && <p className="hint">{t("prov.modelsEmpty")}</p>}
                    {form.models.length > 0 && (
                      <div className="prov-models__row prov-models__grid prov-models__head" style={{ gridTemplateColumns: columns }}>
                        <span>{t("prov.modelId")}</span>
                        <span>{t("prov.modelWindow")}</span>
                        <span>{t("prov.modelEffort")}</span>
                        {form.format === "anthropic" && <span>{t("prov.thinking")}</span>}
                        <span />
                      </div>
                    )}
                    {form.models.map((model) => {
                      const tuning = form.tuning[model] ?? tuningOf(undefined);
                      const tune = (change: Partial<ModelTuning>) => setForm((current) => ({ ...current, tuning: { ...current.tuning, [model]: { ...tuning, ...change } } }));
                      const invalid = parseWindows(tuning.windows) == null;
                      return (
                        <div key={model} className="prov-models__row prov-models__grid" style={{ gridTemplateColumns: columns }}>
                          <span title={model}>{model}</span>
                          <input aria-label={`${t("prov.modelWindow")} ${model}`} className={invalid ? "invalid" : ""} value={tuning.windows} placeholder="128K, 1M" onChange={(event) => tune({ windows: event.target.value })} />
                          <select aria-label={`${t("prov.modelEffort")} ${model}`} value={tuning.effort} onChange={(event) => tune({ effort: event.target.value as ModelTuning["effort"] })}>
                            <option value="auto">{t("prov.effortAuto")}</option>
                            <option value="on">{t("prov.effortOn")}</option>
                            <option value="off">{t("prov.effortOff")}</option>
                          </select>
                          {form.format === "anthropic" && (
                            <select aria-label={`${t("prov.thinking")} ${model}`} value={tuning.thinking} onChange={(event) => tune({ thinking: event.target.value as ModelTuning["thinking"] })}>
                              <option value="auto">{t("prov.effortAuto")}</option>
                              <option value="adaptive">{t("prov.thinkingAdaptive")}</option>
                              <option value="budget">{t("prov.thinkingBudget")}</option>
                            </select>
                          )}
                          <button type="button" className="ghost" onClick={() => setForm({ ...form, models: form.models.filter((item) => item !== model) })}>×</button>
                        </div>
                      );
                    })}
                    {form.models.length > 0 && <p className="hint">{t("prov.modelSettingsHint")}</p>}
                    <div className="prov-models__add">
                      <input value={draftModel} placeholder={t("prov.modelId")} onChange={(event) => setDraftModel(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); addDraftModel(); } }} />
                      <button type="button" className="ghost" onClick={addDraftModel}>{t("prov.addModel")}</button>
                      <button type="button" className="ghost" disabled={fetching} onClick={() => void onFetch()}>{fetching ? t("prov.fetching") : t("prov.fetch")}</button>
                    </div>
                  </div>
                  {remote.length > 0 && (
                    <div className="prov-remote">
                      {remote.map((model) => (
                        <button key={model} type="button" className="prov-presets__chip" onClick={() => setForm((current) => current.models.includes(model) ? current : { ...current, models: [...current.models, model], tuning: { ...current.tuning, [model]: current.tuning[model] ?? tuningOf(undefined) } })}>{model}</button>
                      ))}
                    </div>
                  )}
                </div>
                <label className="checks"><input type="checkbox" checked={form.enabled} onChange={(event) => setForm({ ...form, enabled: event.target.checked })} />{t("prov.enabled")}</label>
              </div>
              <div className="prov-form__actions">
                <button type="button" className="primary" disabled={busy} onClick={() => void onSave()}>{t("prov.save")}</button>
                {form.existing && <button type="button" className="ghost" disabled={busy} onClick={() => void onTest(form.id, form.models[0])}>{t("prov.test")}</button>}
                {form.existing && <button type="button" className="danger" disabled={busy} onClick={() => void onDelete()}>{t("prov.delete")}</button>}
              </div>
              {notice && <p className="hint">{notice}</p>}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

function OfficialAccount(props: { t: (key: Key) => string; setError: (error: string | null) => void }) {
  const { t } = props;
  const [account, setAccount] = useState<GrokAccount | null>(null);
  const [phase, setPhase] = useState<"idle" | "working">("idle");
  const [loginUrl, setLoginUrl] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let stop = false;
    void (async () => {
      try {
        let next = await grokAccount();
        if (stop) return;
        if (next.signedIn && next.expired && next.hasRefresh && desktopShell()) {
          try {
            next = await grokRefresh();
          } catch (reason) {
            if (stop) return;
            setFailed(true);
            setMessage(String(reason));
          }
        }
        if (!stop) setAccount(next);
      } catch (reason) {
        if (stop) return;
        setFailed(true);
        setMessage(String(reason));
      }
    })();
    return () => {
      stop = true;
    };
  }, []);

  async function onLogin() {
    setFailed(false);
    setMessage(null);
    setLoginUrl(null);
    if (!desktopShell()) {
      setFailed(true);
      setMessage(t("prov.loginDesktopOnly"));
      return;
    }
    setPhase("working");
    try {
      const next = await grokLogin(setLoginUrl);
      setAccount(next);
      setMessage(next.signedIn ? t("prov.loginOk") : t("prov.signedOut"));
      setFailed(!next.signedIn);
    } catch (reason) {
      setFailed(true);
      setMessage(String(reason));
    } finally {
      setPhase("idle");
    }
  }

  async function onLogout() {
    setFailed(false);
    setMessage(null);
    setPhase("working");
    try {
      setAccount(await grokLogout());
      setMessage(t("prov.logoutOk"));
    } catch (reason) {
      setFailed(true);
      setMessage(String(reason));
      props.setError(String(reason));
    } finally {
      setPhase("idle");
    }
  }

  async function onRefresh() {
    setFailed(false);
    setMessage(null);
    setPhase("working");
    try {
      const next = await grokRefresh();
      setAccount(next);
      setMessage(t("prov.refreshOk"));
    } catch (reason) {
      setFailed(true);
      setMessage(String(reason));
    } finally {
      setPhase("idle");
    }
  }

  const who = account?.displayName || account?.email;
  return (
    <div className="prov-account" data-testid="grok-account">
      {account?.signedIn ? (
        <>
          <div className="prov-account__name" data-testid="grok-account-name">{t("prov.signedIn")}{who ? ` · ${who}` : ""}</div>
          {account.email && account.displayName && <p className="hint">{account.email}</p>}
          {account.expired && <p className="prov-error" role="alert">{t("prov.expired")}</p>}
          <div className="prov-form__actions">
            <button type="button" className="ghost" data-testid="grok-refresh" disabled={phase === "working"} onClick={() => void onRefresh()}>{t("prov.refresh")}</button>
            <button type="button" className="ghost" data-testid="grok-logout" disabled={phase === "working"} onClick={() => void onLogout()}>{t("prov.logout")}</button>
          </div>
        </>
      ) : (
        <>
          <div className="prov-account__name">{t("prov.signedOut")}</div>
          <p className="hint">{t("prov.loginHint")}</p>
          <div className="prov-form__actions">
            {phase === "working" ? (
              <button type="button" className="ghost" onClick={() => void grokLoginCancel()}>{t("prov.loginCancel")}</button>
            ) : (
              <button type="button" className="primary" data-testid="grok-login" onClick={() => void onLogin()}>{t("prov.login")}</button>
            )}
          </div>
          {phase === "working" && <p className="hint">{t("prov.loginWorking")}</p>}
        </>
      )}
      {loginUrl && (
        <button type="button" className="prov-link" onClick={() => void openExternalUrl(loginUrl).catch((reason: unknown) => { setFailed(true); setMessage(String(reason)); })}>{t("prov.loginUrl")}</button>
      )}
      <p className="hint">{t("prov.oauthFirst")}</p>
      {message && <p className={failed ? "prov-error" : "hint"} role={failed ? "alert" : "status"}>{message}</p>}
    </div>
  );
}

function GrokKey(props: { t: (key: Key) => string; onSave: (secret: string) => Promise<void>; setError: (error: string | null) => void }) {
  const [value, setValue] = useState("");
  const [show, setShow] = useState(false);
  return (
    <label className="field">{props.t("prov.apiKey")}
      <span className="prov-key-row">
        <input type={show ? "text" : "password"} autoComplete="off" value={value} placeholder={props.t("prov.keyKeep")} onChange={(event) => setValue(event.target.value)} />
        <button type="button" className="ghost" onClick={() => setShow((current) => !current)}>{show ? props.t("prov.keyHide") : props.t("prov.keyShow")}</button>
        <button type="button" className="ghost" disabled={!value.trim()} onClick={() => void props.onSave(value.trim()).then(() => setValue("")).catch((reason: unknown) => props.setError(String(reason)))}>{props.t("settings.saveKey")}</button>
      </span>
    </label>
  );
}
