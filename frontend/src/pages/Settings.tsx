import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  KeyRound,
  PlugZap,
  RefreshCw,
  Save,
  ShieldCheck,
  Trash2,
  Tv,
} from "lucide-react";
import {
  api,
  type PlexServerOption,
  type TestConnectionField,
  type UpdateSettingsBody,
} from "@/lib/api";
import { LangField } from "./Media";

type Overwrite = "overwrite" | "suffix" | "skip";

const inputCls =
  "rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent";

export default function Settings() {
  const queryClient = useQueryClient();
  const { data, isPending, isError, error, refetch } = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.getSettings(),
  });

  const [targetLang, setTargetLang] = useState("");
  const [outputPattern, setOutputPattern] = useState("");
  const [overwrite, setOverwrite] = useState<Overwrite>("suffix");
  const [concurrency, setConcurrency] = useState(2);
  const [apiKeys, setApiKeys] = useState<Record<string, string>>({});
  const [clearKeys, setClearKeys] = useState<Set<string>>(new Set());
  const [authToken, setAuthToken] = useState("");
  const [clearAuth, setClearAuth] = useState(false);

  const [customServer, setCustomServer] = useState("");
  const [customEndpoint, setCustomEndpoint] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [customModelsUrl, setCustomModelsUrl] = useState("");
  const [customChat, setCustomChat] = useState(true);
  const [plexServer, setPlexServer] = useState("");
  const [plexConnecting, setPlexConnecting] = useState(false);
  const [plexConnectError, setPlexConnectError] = useState<string | null>(null);
  const [plexServers, setPlexServers] = useState<PlexServerOption[]>([]);
  const [plexServersLoading, setPlexServersLoading] = useState(false);
  const [plexServersError, setPlexServersError] = useState<string | null>(null);
  const [plexTesting, setPlexTesting] = useState(false);
  const [plexTestMessage, setPlexTestMessage] = useState<{
    kind: "ok" | "error";
    text: string;
    field: TestConnectionField;
  } | null>(null);
  const [plexRemoving, setPlexRemoving] = useState(false);
  const [jellyfinServer, setJellyfinServer] = useState("");
  const [jellyfinToken, setJellyfinToken] = useState("");
  const [clearJellyfin, setClearJellyfin] = useState(false);
  const [jellyfinTesting, setJellyfinTesting] = useState(false);
  const [jellyfinTestMessage, setJellyfinTestMessage] = useState<{
    kind: "ok" | "error";
    text: string;
    field: TestConnectionField;
  } | null>(null);
  const [jellyfinRemoving, setJellyfinRemoving] = useState(false);
  const [customModels, setCustomModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  // Populate the non-secret fields from the loaded settings. Secret fields
  // (API keys, token) stay blank — they are write-only and masked on read.
  useEffect(() => {
    if (!data) return;
    setTargetLang(data.default_target_language);
    setOutputPattern(data.output_pattern);
    setOverwrite(data.overwrite_behavior);
    setConcurrency(data.concurrency);
    setCustomServer(data.custom_server_url);
    setCustomEndpoint(data.custom_endpoint);
    setCustomModel(data.custom_model);
    setCustomModelsUrl(data.custom_models_url);
    setCustomChat(data.custom_chat);
    setPlexServer(data.plex_server_url);
    setJellyfinServer(data.jellyfin_server_url);
    setCustomModels([]);
    setModelsError(null);
  }, [data]);

  async function refreshPlexServers() {
    setPlexServersLoading(true);
    setPlexServersError(null);
    try {
      const res = await api.plexServers();
      setPlexServers(res.servers);
      if (res.servers.length === 1) {
        setPlexServer(res.servers[0].url);
      }
    } catch (e) {
      setPlexServers([]);
      setPlexServersError(e instanceof Error ? e.message : "Failed to list Plex servers");
    } finally {
      setPlexServersLoading(false);
    }
  }

  async function connectPlex() {
    setPlexConnecting(true);
    setPlexConnectError(null);
    setPlexServers([]);
    setPlexServersError(null);
    try {
      const start = await api.plexConnect();
      const popup = window.open(start.auth_url, "plex-auth", "width=520,height=680");
      if (!popup) {
        setPlexConnectError("Popup blocked. Allow popups for this page and try again.");
        return;
      }

      for (let attempt = 0; attempt < 90; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, 2000));
        const poll = await api.plexPoll(start.pin_id);
        if (poll.authorized) {
          popup.close();
          await queryClient.invalidateQueries({ queryKey: ["settings"] });
          await refreshPlexServers();
          return;
        }
      }
      setPlexConnectError("Plex approval timed out. Try again.");
    } catch (e) {
      setPlexConnectError(e instanceof Error ? e.message : "Failed to connect with Plex");
    } finally {
      setPlexConnecting(false);
    }
  }

  async function testPlex() {
    setPlexTesting(true);
    setPlexTestMessage(null);
    try {
      const res = await api.testPlex({ server_url: plexServer.trim() });
      setPlexTestMessage({
        kind: res.ok ? "ok" : "error",
        text: res.message,
        field: res.field,
      });
    } catch (e) {
      setPlexTestMessage({
        kind: "error",
        text: e instanceof Error ? e.message : "Plex connection test failed",
        field: "general",
      });
    } finally {
      setPlexTesting(false);
    }
  }

  async function removePlex() {
    setPlexRemoving(true);
    setPlexTestMessage(null);
    try {
      await api.removePlex();
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      setPlexServer("");
      setPlexServers([]);
      setPlexServersError(null);
      setPlexConnectError(null);
      setPlexTestMessage({
        kind: "ok",
        text: "Plex configuration removed",
        field: "server_url",
      });
    } catch (e) {
      setPlexTestMessage({
        kind: "error",
        text: e instanceof Error ? e.message : "Failed to remove Plex configuration",
        field: "general",
      });
    } finally {
      setPlexRemoving(false);
    }
  }

  async function testJellyfin() {
    setJellyfinTesting(true);
    setJellyfinTestMessage(null);
    try {
      const res = await api.testJellyfin({
        server_url: jellyfinServer.trim(),
        token: jellyfinToken.trim() || undefined,
      });
      setJellyfinTestMessage({
        kind: res.ok ? "ok" : "error",
        text: res.message,
        field: res.field,
      });
    } catch (e) {
      setJellyfinTestMessage({
        kind: "error",
        text: e instanceof Error ? e.message : "Jellyfin connection test failed",
        field: "general",
      });
    } finally {
      setJellyfinTesting(false);
    }
  }

  async function removeJellyfin() {
    setJellyfinRemoving(true);
    setJellyfinTestMessage(null);
    try {
      await api.removeJellyfin();
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      setJellyfinServer("");
      setJellyfinToken("");
      setClearJellyfin(false);
      setJellyfinTestMessage({
        kind: "ok",
        text: "Jellyfin configuration removed",
        field: "server_url",
      });
    } catch (e) {
      setJellyfinTestMessage({
        kind: "error",
        text: e instanceof Error ? e.message : "Failed to remove Jellyfin configuration",
        field: "general",
      });
    } finally {
      setJellyfinRemoving(false);
    }
  }

  async function save() {
    setSaving(true);
    setSaved(false);
    setSubmitError(null);
    try {
      const body: UpdateSettingsBody = {
        default_target_language: targetLang,
        output_pattern: outputPattern,
        overwrite_behavior: overwrite,
        concurrency,
        custom_server_url: customServer.trim(),
        custom_endpoint: customEndpoint.trim(),
        custom_model: customModel.trim(),
        custom_models_url: customModelsUrl.trim(),
        custom_chat: customChat,
        plex_server_url: plexServer.trim(),
        jellyfin_server_url: jellyfinServer.trim(),
      };
      if (jellyfinToken.trim() !== "") body.jellyfin_token = jellyfinToken;
      else if (clearJellyfin) body.jellyfin_token = "";
      const api_keys: Record<string, string> = {};
      for (const [id, val] of Object.entries(apiKeys)) {
        if (val.trim() !== "") api_keys[id] = val;
      }
      for (const id of clearKeys) api_keys[id] = "";
      if (Object.keys(api_keys).length > 0) body.api_keys = api_keys;
      if (authToken.trim() !== "") body.auth_token = authToken;
      else if (clearAuth) body.auth_token = "";

      await api.putSettings(body);
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      // Reset the write-only secret inputs (they only ever send new values).
      setApiKeys({});
      setClearKeys(new Set());
      setAuthToken("");
      setClearAuth(false);
      setJellyfinToken("");
      setClearJellyfin(false);
      setCustomModels([]);
      setModelsError(null);
      setSaved(true);
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : "Failed to save settings");
    } finally {
      setSaving(false);
    }
  }

  async function refreshCustomModels() {
    setModelsLoading(true);
    setModelsError(null);
    try {
      const res = await api.listModels("custom");
      if (!res.supports_list) {
        setCustomModels([]);
        setModelsError("Save a server URL first, then refresh models.");
      } else if (res.error) {
        setCustomModels([]);
        setModelsError(res.error);
      } else {
        setCustomModels(res.models);
      }
    } catch (e) {
      setCustomModels([]);
      setModelsError(e instanceof Error ? e.message : "Failed to list models");
    } finally {
      setModelsLoading(false);
    }
  }

  const customProvider = data?.providers.find((p) => p.id === "custom");
  const customModelOptions =
    customModels.length > 0
      ? customModel && !customModels.includes(customModel)
        ? [customModel, ...customModels]
        : customModels
      : [];

  if (isPending) {
    return (
      <div>
        <h1 className="mb-4 text-xl font-semibold">Settings</h1>
        <p className="text-sm text-muted">Loading…</p>
      </div>
    );
  }

  if (isError) {
    return (
      <div>
        <h1 className="mb-4 text-xl font-semibold">Settings</h1>
        <div className="rounded-xl border border-danger/40 bg-danger/10 p-4 text-sm text-danger">
          <span>
            Failed to load settings:{" "}
            {error instanceof Error ? error.message : String(error)}
          </span>
          <button onClick={() => refetch()} className="ml-3 underline">
            Retry
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-2xl">
      <h1 className="mb-1 text-xl font-semibold">Settings</h1>
      <p className="mb-5 text-sm text-muted">
        Provider credentials, translation defaults, and access control.
      </p>

      <div className="space-y-5">
        {/* Provider API keys */}
        <section className="rounded-xl border border-border bg-surface p-5">
          <div className="mb-4 flex items-center gap-2">
            <KeyRound size={16} className="text-muted" />
            <h2 className="text-sm font-semibold">Provider API keys</h2>
          </div>
          <div className="space-y-3">
            {data.providers.map((p) => (
              <div
                key={p.id}
                className="flex flex-col gap-1 sm:flex-row sm:items-center sm:gap-3"
              >
                <div className="w-48 shrink-0">
                  <p className="text-sm">{p.label}</p>
                  {!p.needs_key && (
                    <p className="text-xs text-muted">no key required</p>
                  )}
                </div>
                {p.needs_key && (
                  <div className="flex flex-1 items-center gap-2">
                    <input
                      type="password"
                      value={apiKeys[p.id] ?? ""}
                      onChange={(e) => {
                        setApiKeys((prev) => ({ ...prev, [p.id]: e.target.value }));
                        setClearKeys((prev) => {
                          const next = new Set(prev);
                          next.delete(p.id);
                          return next;
                        });
                      }}
                      placeholder={p.key_set ? p.key_masked ?? "•••• (set)" : "not set"}
                      autoComplete="new-password"
                      className={inputCls + " w-full"}
                    />
                    {p.key_set && !clearKeys.has(p.id) && (
                      <button
                        type="button"
                        title="Remove key"
                        onClick={() => {
                          setClearKeys((prev) => new Set(prev).add(p.id));
                          setApiKeys((prev) => ({ ...prev, [p.id]: "" }));
                        }}
                        className="rounded-lg border border-border p-2 text-muted hover:border-danger hover:text-danger"
                      >
                        <Trash2 size={14} />
                      </button>
                    )}
                    {clearKeys.has(p.id) && (
                      <span className="text-xs text-danger">will be removed</span>
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        </section>

        {/* Custom OpenAI-compatible API */}
        <section className="rounded-xl border border-border bg-surface p-5">
          <div className="mb-4 flex items-center gap-2">
            <PlugZap size={16} className="text-muted" />
            <h2 className="text-sm font-semibold">
              Custom AI API (OpenAI-compatible)
            </h2>
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Server URL</span>
              <input
                value={customServer}
                onChange={(e) => setCustomServer(e.target.value)}
                placeholder="http://localhost:11434"
                className={inputCls}
              />
              <span className="text-xs text-muted">
                Address only; do not include the endpoint path.
              </span>
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Endpoint</span>
              <input
                value={customEndpoint}
                onChange={(e) => setCustomEndpoint(e.target.value)}
                placeholder="/v1/chat/completions"
                className={inputCls}
              />
              <span className="text-xs text-muted">
                Defaults to /v1/chat/completions when blank.
              </span>
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">API token (optional)</span>
              <div className="flex items-center gap-2">
                <input
                  type="password"
                  value={apiKeys["custom"] ?? ""}
                  onChange={(e) => {
                    setApiKeys((prev) => ({ ...prev, custom: e.target.value }));
                    setClearKeys((prev) => {
                      const next = new Set(prev);
                      next.delete("custom");
                      return next;
                    });
                  }}
                  placeholder={
                    customProvider?.key_set
                      ? customProvider.key_masked ?? "•••• (set)"
                      : "not required"
                  }
                  autoComplete="new-password"
                  className={inputCls + " w-full"}
                />
                {customProvider?.key_set && !clearKeys.has("custom") && (
                  <button
                    type="button"
                    title="Remove token"
                    onClick={() => {
                      setClearKeys((prev) => new Set(prev).add("custom"));
                      setApiKeys((prev) => ({ ...prev, custom: "" }));
                    }}
                    className="rounded-lg border border-border p-2 text-muted hover:border-danger hover:text-danger"
                  >
                    <Trash2 size={14} />
                  </button>
                )}
                {clearKeys.has("custom") && (
                  <span className="text-xs text-danger">will be removed</span>
                )}
              </div>
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Models URL (optional)</span>
              <input
                value={customModelsUrl}
                onChange={(e) => setCustomModelsUrl(e.target.value)}
                placeholder="Auto-detect (/v1/models)"
                className={inputCls}
              />
            </label>
            <div className="flex flex-col gap-1 text-sm sm:col-span-2">
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted">Model</span>
                <button
                  type="button"
                  onClick={refreshCustomModels}
                  disabled={modelsLoading}
                  className="inline-flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-xs text-muted hover:border-accent hover:text-foreground disabled:opacity-50"
                >
                  <RefreshCw
                    size={12}
                    className={modelsLoading ? "animate-spin" : ""}
                  />
                  {modelsLoading ? "Loading…" : "Refresh models"}
                </button>
              </div>
              {customModelOptions.length > 0 ? (
                <select
                  value={customModel}
                  onChange={(e) => setCustomModel(e.target.value)}
                  className={inputCls}
                >
                  <option value="">Auto (server default)</option>
                  {customModelOptions.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  value={customModel}
                  onChange={(e) => setCustomModel(e.target.value)}
                  placeholder="e.g. llama3.1"
                  className={inputCls}
                />
              )}
              <span className="text-xs text-muted">
                Save first, then refresh to list models from the server.
              </span>
              {modelsError && (
                <span className="text-xs text-warning">{modelsError}</span>
              )}
            </div>
            <label className="flex cursor-pointer items-center gap-2 text-sm sm:col-span-2">
              <input
                type="checkbox"
                checked={customChat}
                onChange={(e) => setCustomChat(e.target.checked)}
                className="accent-[var(--color-accent)]"
              />
              Use chat-format requests
            </label>
          </div>
        </section>

        {/* Translation defaults */}
        <section className="rounded-xl border border-border bg-surface p-5">
          <h2 className="mb-4 text-sm font-semibold">Translation defaults</h2>
          <div className="grid gap-4 sm:grid-cols-2">
            <LangField
              value={targetLang}
              onChange={setTargetLang}
              label="Default target language"
            />
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Concurrency (max jobs)</span>
              <input
                type="number"
                min={1}
                value={concurrency}
                onChange={(e) => setConcurrency(Number(e.target.value) || 1)}
                className={inputCls}
              />
              <span className="text-xs text-muted">Applies on restart.</span>
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Output naming pattern</span>
              <input
                value={outputPattern}
                onChange={(e) => setOutputPattern(e.target.value)}
                placeholder="{name}.{lang}.srt"
                className={inputCls}
              />
              <span className="text-xs text-muted">
                Placeholders: {`{name} {lang} {ext}`}
              </span>
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Overwrite behavior</span>
              <select
                value={overwrite}
                onChange={(e) => setOverwrite(e.target.value as Overwrite)}
                className={inputCls}
              >
                <option value="suffix">Suffix (-2, -3, …)</option>
                <option value="overwrite">Overwrite</option>
                <option value="skip">Skip</option>
              </select>
            </label>
          </div>
        </section>

        {/* Access control */}
        <section className="rounded-xl border border-border bg-surface p-5">
          <div className="mb-4 flex items-center gap-2">
            <ShieldCheck size={16} className="text-muted" />
            <h2 className="text-sm font-semibold">Access control</h2>
          </div>
          <label className="flex flex-col gap-1 text-sm">
            <span className="text-muted">Shared access token</span>
            <div className="flex items-center gap-2">
              <input
                type="password"
                value={authToken}
                onChange={(e) => {
                  setAuthToken(e.target.value);
                  setClearAuth(false);
                }}
                placeholder={
                  data.auth_token_set
                    ? data.auth_token_masked ?? "•••• (set)"
                    : "not set (open access)"
                }
                autoComplete="new-password"
                className={inputCls + " w-full"}
              />
              {data.auth_token_set && !clearAuth && (
                <button
                  type="button"
                  title="Remove token"
                  onClick={() => {
                    setClearAuth(true);
                    setAuthToken("");
                  }}
                  className="rounded-lg border border-border p-2 text-muted hover:border-danger hover:text-danger"
                >
                  <Trash2 size={14} />
                </button>
              )}
              {clearAuth && (
                <span className="text-xs text-danger">will be removed</span>
              )}
            </div>
            <span className="text-xs text-muted">
              When set, the API requires this token (Authorization: Bearer).
              Leave blank for open access.
            </span>
          </label>
        </section>

        {/* Media server notifications */}
        <section className="rounded-xl border border-border bg-surface p-5">
          <div className="mb-4 flex items-center gap-2">
            <Tv size={16} className="text-muted" />
            <h2 className="text-sm font-semibold">Media server notifications</h2>
          </div>
          <p className="mb-4 text-xs text-muted">
            When a translation job finishes, TranSFlator asks the configured
            server to refresh the matching movie or episode so the new subtitle
            appears without a manual library scan.
          </p>

          <div className="mb-4">
            <span className="mb-1 block text-sm text-muted">Plex server URL</span>
            <div className="flex flex-wrap items-center gap-2">
              <input
                value={plexServer}
                onChange={(e) => {
                  setPlexServer(e.target.value);
                  setPlexTestMessage(null);
                }}
                placeholder="http://localhost:32400"
                className={inputCls + " min-w-0 flex-1"}
              />
              <button
                type="button"
                onClick={connectPlex}
                disabled={plexConnecting}
                className="inline-flex items-center gap-2 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-2 disabled:opacity-50"
              >
                <PlugZap size={14} className={plexConnecting ? "animate-pulse" : ""} />
                {plexConnecting
                  ? "Waiting for Plex approval…"
                  : data.plex_token_set
                    ? "Reconnect with Plex"
                    : "Connect with Plex"}
              </button>
              <button
                type="button"
                onClick={testPlex}
                disabled={plexTesting || plexConnecting}
                className="inline-flex items-center gap-1 rounded-lg border border-border px-3 py-2 text-sm text-muted hover:border-accent hover:text-foreground disabled:opacity-50"
              >
                {plexTesting ? "Testing…" : "Test"}
              </button>
              {(data.plex_server_url !== "" || data.plex_token_set) && (
                <button
                  type="button"
                  onClick={removePlex}
                  disabled={plexRemoving}
                  className="inline-flex items-center gap-1 rounded-lg border border-border px-3 py-2 text-sm text-muted hover:border-danger hover:text-danger disabled:opacity-50"
                >
                  <Trash2 size={14} />
                  {plexRemoving ? "Removing…" : "Remove"}
                </button>
              )}
              {data.plex_token_set && (
                <button
                  type="button"
                  onClick={refreshPlexServers}
                  disabled={plexServersLoading}
                  className="inline-flex items-center gap-1 rounded-lg border border-border px-3 py-2 text-sm text-muted hover:border-accent hover:text-foreground disabled:opacity-50"
                >
                  <RefreshCw size={14} className={plexServersLoading ? "animate-spin" : ""} />
                  {plexServersLoading ? "Loading…" : "Refresh servers"}
                </button>
              )}
            </div>
            {plexTestMessage && (
              <p
                className={
                  plexTestMessage.kind === "ok"
                    ? "mt-2 text-xs text-success"
                    : "mt-2 text-xs text-danger"
                }
              >
                {plexTestMessage.text}
              </p>
            )}
            {plexConnectError && (
              <p className="mt-2 text-xs text-danger">{plexConnectError}</p>
            )}
            {plexServersError && (
              <p className="mt-2 text-xs text-warning">{plexServersError}</p>
            )}
            {plexServers.length > 0 && (
              <label className="mt-3 flex flex-col gap-1 text-sm">
                <span className="text-muted">Discovered Plex server</span>
                <select
                  value={plexServer}
                  onChange={(e) => setPlexServer(e.target.value)}
                  className={inputCls}
                >
                  <option value="">Manual entry</option>
                  {plexServer && !plexServers.some((s) => s.url === plexServer) && (
                    <option value={plexServer}>{plexServer}</option>
                  )}
                  {plexServers.map((s) => (
                    <option key={s.url} value={s.url}>
                      {s.name} — {s.hints.join(", ")}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Jellyfin server URL</span>
              <div className="flex flex-wrap items-center gap-2">
                <input
                  value={jellyfinServer}
                  onChange={(e) => {
                    setJellyfinServer(e.target.value);
                    setJellyfinTestMessage(null);
                  }}
                  placeholder="http://localhost:8096"
                  className={inputCls + " min-w-0 flex-1"}
                />
                {(data.jellyfin_server_url !== "" || data.jellyfin_token_set) && (
                  <button
                    type="button"
                    onClick={removeJellyfin}
                    disabled={jellyfinRemoving}
                    className="inline-flex items-center gap-1 rounded-lg border border-border px-3 py-2 text-sm text-muted hover:border-danger hover:text-danger disabled:opacity-50"
                  >
                    <Trash2 size={14} />
                    {jellyfinRemoving ? "Removing…" : "Remove"}
                  </button>
                )}
              </div>
              {jellyfinTestMessage?.field === "server_url" && (
                <p
                  className={
                    jellyfinTestMessage.kind === "ok"
                      ? "text-xs text-success"
                      : "text-xs text-danger"
                  }
                >
                  {jellyfinTestMessage.text}
                </p>
              )}
            </div>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-muted">Jellyfin API key</span>
              <div className="flex items-center gap-2">
                <input
                  type="password"
                  value={jellyfinToken}
                  onChange={(e) => {
                    setJellyfinToken(e.target.value);
                    setClearJellyfin(false);
                    setJellyfinTestMessage(null);
                  }}
                  placeholder={
                    data.jellyfin_token_set
                      ? data.jellyfin_token_masked ?? "•••• (set)"
                      : "not set"
                  }
                  autoComplete="new-password"
                  className={inputCls + " min-w-0 flex-1"}
                />
                <button
                  type="button"
                  onClick={testJellyfin}
                  disabled={jellyfinTesting}
                  className="inline-flex items-center gap-1 rounded-lg border border-border px-3 py-2 text-sm text-muted hover:border-accent hover:text-foreground disabled:opacity-50"
                >
                  {jellyfinTesting ? "Testing…" : "Test"}
                </button>
                {data.jellyfin_token_set && !clearJellyfin && (
                  <button
                    type="button"
                    title="Remove Jellyfin API key"
                    onClick={() => {
                      setClearJellyfin(true);
                      setJellyfinToken("");
                    }}
                    className="rounded-lg border border-border p-2 text-muted hover:border-danger hover:text-danger"
                  >
                    <Trash2 size={14} />
                  </button>
                )}
                {clearJellyfin && (
                  <span className="text-xs text-danger">will be removed</span>
                )}
              </div>
              {jellyfinTestMessage && jellyfinTestMessage.field !== "server_url" && (
                <p
                  className={
                    jellyfinTestMessage.kind === "ok"
                      ? "text-xs text-success"
                      : "text-xs text-danger"
                  }
                >
                  {jellyfinTestMessage.text}
                </p>
              )}
            </label>
          </div>
        </section>

        {/* Save */}
        <div className="flex items-center gap-3">
          <button
            onClick={save}
            disabled={saving}
            className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-2 disabled:opacity-50"
          >
            <Save size={14} />
            {saving ? "Saving…" : "Save settings"}
          </button>
          {saved && (
            <span className="inline-flex items-center gap-1 text-sm text-success">
              <CheckCircle2 size={14} /> Saved
            </span>
          )}
        </div>
        {submitError && <p className="text-xs text-danger">{submitError}</p>}
      </div>
    </div>
  );
}
