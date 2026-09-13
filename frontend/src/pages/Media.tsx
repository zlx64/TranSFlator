import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  Languages,
  RefreshCw,
  Send,
  Upload,
} from "lucide-react";
import {
  api,
  LANGUAGES,
  PROVIDERS,
  type Stream,
  type StreamsResponse,
} from "@/lib/api";
import { cn } from "@/lib/utils";

function formatDuration(s: number | null): string {
  if (s === null) return "—";
  const total = Math.round(s);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

function formatSize(bytes?: number): string {
  if (bytes === undefined) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes;
  let u = -1;
  do {
    v /= 1024;
    u++;
  } while (v >= 1024 && u < units.length - 1);
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[u]}`;
}

/** Mirror of the backend `Stream::display_label` (§6.7). */
function streamLabel(s: Stream): string {
  const parts: string[] = [];
  if (s.language) parts.push(s.language);
  if (s.title && s.title.trim()) parts.push(s.title);
  const flags: string[] = [];
  if (s.default) flags.push("default");
  if (s.forced) flags.push("forced");
  if (s.subtitle_kind)
    flags.push(s.subtitle_kind === "text" ? "text" : "image — OCR required");
  let label = `#${s.index}`;
  if (parts.length > 0) label += ` — ${parts.join(", ")}`;
  if (flags.length > 0) label += ` (${flags.join(", ")})`;
  return label;
}

export default function Media() {
  const [params] = useSearchParams();
  const root = Number(params.get("root") ?? 0);
  const path = params.get("path") ?? "";
  const fileName = path.split("/").filter(Boolean).pop() ?? path;

  // Incrementing tick forces a re-scan (bypasses the ffprobe cache).
  const [forceTick, setForceTick] = useState(0);
  // User's explicit picker choice; null = follow auto/preferred.
  const [override, setOverride] = useState<number | null>(null);

  // Translate job form (Phase 5).
  const navigate = useNavigate();
  const [formOpen, setFormOpen] = useState(false);
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("");
  const [targetLang, setTargetLang] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  // §6.1: external subtitle upload for files without subtitle tracks.
  const [uploadFile, setUploadFile] = useState<File | null>(null);

  // Remembered provider/model/language — pre-fill the form once on load.
  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.getSettings(),
    staleTime: 5 * 60 * 1000,
  });
  // Available models for the selected provider (dropdown when listable).
  const modelsQuery = useQuery({
    queryKey: ["models", provider],
    queryFn: () => api.listModels(provider),
    staleTime: 5 * 60 * 1000,
  });
  const prefilled = useRef(false);
  useEffect(() => {
    if (prefilled.current) return;
    const s = settingsQuery.data;
    if (!s) return;
    prefilled.current = true;
    if (
      s.default_provider &&
      PROVIDERS.some((p) => p.id === s.default_provider)
    ) {
      setProvider(s.default_provider);
    }
    if (s.default_model) setModel(s.default_model);
    if (s.default_target_language) setTargetLang(s.default_target_language);
  }, [settingsQuery.data]);

  // Persist the current selection so it pre-fills next time (best-effort).
  async function rememberSelection() {
    await api
      .putSettings({
        default_provider: provider,
        default_model: model.trim(),
        default_target_language: targetLang.trim(),
      })
      .catch(() => {});
  }

  async function submitUpload() {
    if (!uploadFile) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      await api.uploadSubtitleJob(uploadFile, {
        root,
        path,
        provider,
        target_language: targetLang.trim() || undefined,
        model: model.trim() || undefined,
      });
      await rememberSelection();
      navigate("/jobs");
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : "upload failed");
      setSubmitting(false);
    }
  }

  async function submitJob() {
    setSubmitting(true);
    setSubmitError(null);
    try {
      await api.createJob({
        root,
        path,
        stream_index: pickedIndex ?? undefined,
        target_language: targetLang.trim() || undefined,
        provider,
        model: model.trim() || undefined,
      });
      await rememberSelection();
      navigate("/jobs");
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : "failed to start job");
      setSubmitting(false);
    }
  }

  const {
    data,
    isPending,
    isRefetching,
    error,
  } = useQuery({
    queryKey: ["media", "streams", root, path, forceTick],
    queryFn: () =>
      api.get<StreamsResponse>("/api/media/streams", {
        root,
        path,
        force: forceTick > 0,
      }),
    enabled: path.length > 0,
  });

  useEffect(() => {
    setOverride(null);
    setForceTick(0);
  }, [root, path]);

  const info = data?.info;
  const selection = data?.selection;
  const subs = info?.streams.filter((s) => s.kind === "subtitle") ?? [];

  const isAuto = selection !== undefined && typeof selection === "object";
  const autoIndex = isAuto ? selection.auto.stream_index : null;
  const isPicker = selection === "picker";
  const isImageOnly = selection === "image_only";
  const isNone = selection === "none";
  const preferredIndex = isPicker ? data?.preferred ?? null : null;
  const pickedIndex = override ?? autoIndex ?? preferredIndex;
  const pickedStream = subs.find((s) => s.index === pickedIndex) ?? null;
  const autoStream = info?.streams.find((s) => s.index === autoIndex) ?? null;
  const canTranslate = pickedStream?.subtitle_kind === "text";

  return (
    <div>
      <Link
        to={`/library?root=${root}`}
        className="mb-4 inline-flex items-center gap-1 text-sm text-muted hover:text-foreground"
      >
        <ArrowLeft size={14} /> Back to library
      </Link>

      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h1 className="max-w-[70vw] truncate text-xl font-semibold">
          {fileName}
        </h1>
        <button
          onClick={() => setForceTick((t) => t + 1)}
          disabled={isRefetching}
          className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50"
        >
          <RefreshCw
            size={14}
            className={cn(isRefetching && "animate-spin")}
          />
          Re-scan
        </button>
      </div>

      {isPending && <p className="text-sm text-muted">Probing media…</p>}

      {error && (
        <div className="rounded-lg border border-danger/40 bg-danger/10 p-4 text-sm text-danger">
          {error.message}
        </div>
      )}

      {info && selection && (
        <>
          {/* File meta */}
          <div className="mb-4 flex flex-wrap gap-x-6 gap-y-1 rounded-xl border border-border bg-surface px-4 py-3 text-sm">
            <Meta label="Duration" value={formatDuration(info.duration_s)} />
            <Meta label="Container" value={info.container ?? "—"} />
            <Meta label="Size" value={formatSize(info.size)} />
            <Meta
              label="Streams"
              value={`${info.streams.length} (${info.video_count} video, ${info.audio_count} audio, ${info.subtitle_count} sub)`}
            />
          </div>

          {/* Subtitle selection (FR-7) */}
          <div className="mb-4">
            {isAuto && autoStream && (
              <div className="flex items-start gap-3 rounded-xl border border-success/40 bg-success/10 p-4 text-sm">
                <CheckCircle2 size={18} className="mt-0.5 shrink-0 text-success" />
                <div>
                  <p className="font-medium text-success">
                    Subtitle track auto-selected
                  </p>
                  <p className="mt-0.5 text-foreground">
                    {streamLabel(autoStream)}
                  </p>
                </div>
              </div>
            )}

            {isPicker && (
              <div className="rounded-xl border border-border bg-surface p-4">
                <p className="mb-3 text-sm font-medium">
                  Multiple subtitle tracks found — choose one to translate:
                </p>
                <div className="space-y-2">
                  {subs.map((s) => (
                    <label
                      key={s.index}
                      className={cn(
                        "flex cursor-pointer items-center gap-3 rounded-lg border px-4 py-3 text-sm",
                        pickedIndex === s.index
                          ? "border-accent bg-accent/10"
                          : "border-border bg-surface-2 hover:border-accent/50",
                      )}
                    >
                      <input
                        type="radio"
                        name="subtitle-track"
                        checked={pickedIndex === s.index}
                        onChange={() => setOverride(s.index)}
                        className="accent-[var(--color-accent)]"
                      />
                      <span className="font-medium">{streamLabel(s)}</span>
                      <span className="ml-auto text-xs text-muted">
                        {s.codec}
                      </span>
                    </label>
                  ))}
                </div>
              </div>
            )}

            {isImageOnly && (
              <Notice tone="warning" title="Image-based subtitles only">
                All subtitle tracks in this file are image-based (e.g. PGS /
                VobSub). They cannot be translated directly — OCR is not
                supported. Try a file with text-based subtitles.
              </Notice>
            )}

            {isNone && (
              <div className="rounded-xl border border-warning/40 bg-warning/10 p-4 text-sm">
                <div className="flex items-start gap-3">
                  <AlertTriangle
                    size={18}
                    className="mt-0.5 shrink-0 text-warning"
                  />
                  <div className="min-w-0 flex-1">
                    <p className="font-medium text-warning">
                      No subtitle tracks found
                    </p>
                    <p className="mt-0.5 text-foreground">
                      This file has no embedded subtitle streams. Upload an
                      external .srt / .ass / .vtt file to translate instead:
                    </p>

                    <div className="mt-3 grid gap-3 sm:grid-cols-2">
                      <label className="flex flex-col gap-1">
                        <span className="text-muted">Subtitle file</span>
                        <input
                          type="file"
                          accept=".srt,.ass,.vtt"
                          onChange={(e) =>
                            setUploadFile(e.target.files?.[0] ?? null)
                          }
                          className="w-full text-xs text-muted file:mr-3 file:cursor-pointer file:rounded-lg file:border-0 file:bg-surface-2 file:px-3 file:py-1.5 file:text-sm file:font-medium file:text-foreground hover:file:border-accent"
                        />
                      </label>
                      <LangField
                        value={targetLang}
                        onChange={setTargetLang}
                      />
                      <label className="flex flex-col gap-1">
                        <span className="text-muted">Provider</span>
                        <select
                          value={provider}
                          onChange={(e) => setProvider(e.target.value)}
                          className="rounded-lg border border-border bg-surface-2 px-3 py-2 outline-none focus:border-accent"
                        >
                          {PROVIDERS.map((p) => (
                            <option key={p.id} value={p.id}>
                              {p.label}
                            </option>
                          ))}
                        </select>
                      </label>
                      <ModelField
                        value={model}
                        onChange={setModel}
                        models={modelsQuery.data?.models ?? []}
                        supportsList={modelsQuery.data?.supports_list ?? false}
                        loading={modelsQuery.isPending}
                        error={modelsQuery.data?.error}
                      />
                    </div>

                    {PROVIDERS.find((p) => p.id === provider)?.needsKey && (
                      <p className="mt-2 text-xs text-warning">
                        This provider needs an API key — set it in Settings
                        first.
                      </p>
                    )}

                    {submitError && (
                      <p className="mt-2 text-xs text-danger">{submitError}</p>
                    )}

                    <div className="mt-3 flex items-center gap-3">
                      <button
                        onClick={submitUpload}
                        disabled={submitting || !uploadFile}
                        className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-background hover:bg-accent-2 disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        <Upload size={14} />
                        {submitting ? "Starting…" : "Upload & translate"}
                      </button>
                      <p className="text-xs text-muted">
                        Output is written next to the video file.
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* Translate CTA + job form (Phase 5) */}
          <div className="mb-6">
            <div className="flex flex-wrap items-center gap-3">
              <button
                disabled={!canTranslate}
                onClick={() => {
                  setFormOpen((o) => !o);
                  setSubmitError(null);
                }}
                className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-background transition-colors hover:bg-accent-2 disabled:cursor-not-allowed disabled:opacity-40"
              >
                <Languages size={16} />
                Translate
              </button>
              <p className="text-xs text-muted">
                {pickedStream
                  ? canTranslate
                    ? `${streamLabel(pickedStream)} selected`
                    : "Selected track is image-based; OCR is not supported."
                  : "Select a text subtitle track to translate."}
              </p>
            </div>

            {formOpen && canTranslate && (
              <div className="mt-3 rounded-xl border border-border bg-surface p-4">
                <div className="grid gap-4 sm:grid-cols-3">
                  <label className="flex flex-col gap-1 text-sm">
                    <span className="text-muted">Provider</span>
                    <select
                      value={provider}
                      onChange={(e) => setProvider(e.target.value)}
                      className="rounded-lg border border-border bg-surface-2 px-3 py-2 outline-none focus:border-accent"
                    >
                      {PROVIDERS.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <ModelField
                    value={model}
                    onChange={setModel}
                    models={modelsQuery.data?.models ?? []}
                    supportsList={modelsQuery.data?.supports_list ?? false}
                    loading={modelsQuery.isPending}
                    error={modelsQuery.data?.error}
                  />
                  <LangField value={targetLang} onChange={setTargetLang} />
                </div>

                {PROVIDERS.find((p) => p.id === provider)?.needsKey && (
                  <p className="mt-3 text-xs text-warning">
                    This provider needs an API key — set it in Settings (or the
                    matching env var) before running.
                  </p>
                )}

                {submitError && (
                  <p className="mt-3 text-xs text-danger">{submitError}</p>
                )}

                <div className="mt-4 flex items-center gap-3">
                  <button
                    onClick={submitJob}
                    disabled={submitting}
                    className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-background hover:bg-accent-2 disabled:opacity-50"
                  >
                    <Send size={14} />
                    {submitting ? "Starting…" : "Start translation"}
                  </button>
                  <p className="text-xs text-muted">
                    Output is written next to the source file.
                  </p>
                </div>
              </div>
            )}
          </div>

          {/* All streams */}
          <StreamsTable streams={info.streams} pickedIndex={pickedIndex} />
        </>
      )}
    </div>
  );
}

/** Model input: a dropdown when the provider lists models, otherwise free
 *  text. A custom value not in the fetched list is preserved as an option. */
function ModelField({
  value,
  onChange,
  models,
  supportsList,
  loading,
  error,
}: {
  value: string;
  onChange: (v: string) => void;
  models: string[];
  supportsList: boolean;
  loading: boolean;
  error?: string | null;
}) {
  const useDropdown = supportsList && models.length > 0;
  const options =
    useDropdown && value && !models.includes(value)
      ? [value, ...models]
      : models;
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span className="text-muted">Model (optional)</span>
      {useDropdown ? (
        <select
          value={value}
          onChange={(e) => onChange(e.target.value)}
          disabled={loading}
          className="rounded-lg border border-border bg-surface-2 px-3 py-2 outline-none focus:border-accent"
        >
          <option value="">Auto (provider default)</option>
          {options.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      ) : (
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="e.g. gpt-4o-mini"
          className="rounded-lg border border-border bg-surface-2 px-3 py-2 outline-none focus:border-accent"
        />
      )}
      {error && <span className="text-xs text-warning">{error}</span>}
    </label>
  );
}

/** Target-language dropdown. A remembered value not in the list (e.g. a
 *  custom language name) is preserved as an extra option. */
function LangField({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const options =
    value && !LANGUAGES.includes(value) ? [value, ...LANGUAGES] : LANGUAGES;
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span className="text-muted">Target language</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-lg border border-border bg-surface-2 px-3 py-2 outline-none focus:border-accent"
      >
        <option value="">Auto (default)</option>
        {options.map((l) => (
          <option key={l} value={l}>
            {l}
          </option>
        ))}
      </select>
    </label>
  );
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <span className="text-muted">
      {label} <span className="text-foreground">{value}</span>
    </span>
  );
}

function Notice({
  tone,
  title,
  children,
}: {
  tone: "warning";
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-start gap-3 rounded-xl border p-4 text-sm",
        tone === "warning" && "border-warning/40 bg-warning/10",
      )}
    >
      <AlertTriangle
        size={18}
        className={cn(
          "mt-0.5 shrink-0",
          tone === "warning" && "text-warning",
        )}
      />
      <div>
        <p
          className={cn(
            "font-medium",
            tone === "warning" && "text-warning",
          )}
        >
          {title}
        </p>
        <p className="mt-0.5 text-foreground">{children}</p>
      </div>
    </div>
  );
}

function StreamsTable({
  streams,
  pickedIndex,
}: {
  streams: Stream[];
  pickedIndex: number | null;
}) {
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-surface">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs uppercase tracking-wide text-muted">
            <th className="px-4 py-2 font-medium">#</th>
            <th className="px-4 py-2 font-medium">Type</th>
            <th className="px-4 py-2 font-medium">Codec</th>
            <th className="px-4 py-2 font-medium">Language</th>
            <th className="px-4 py-2 font-medium">Title</th>
            <th className="px-4 py-2 font-medium">Flags</th>
          </tr>
        </thead>
        <tbody>
          {streams.map((s) => {
            const flags: string[] = [];
            if (s.default) flags.push("default");
            if (s.forced) flags.push("forced");
            if (s.subtitle_kind)
              flags.push(s.subtitle_kind === "text" ? "text" : "image");
            return (
              <tr
                key={s.index}
                className={cn(
                  "border-b border-border/50 last:border-0",
                  s.kind === "subtitle" && "bg-accent/5",
                  pickedIndex === s.index && "bg-accent/15",
                )}
              >
                <td className="px-4 py-2 text-muted">{s.index}</td>
                <td className="px-4 py-2">{s.kind}</td>
                <td className="px-4 py-2">{s.codec || "—"}</td>
                <td className="px-4 py-2">{s.language ?? "—"}</td>
                <td className="px-4 py-2">{s.title ?? "—"}</td>
                <td className="px-4 py-2 text-muted">
                  {flags.join(", ") || "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
