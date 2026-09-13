import { useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  ChevronRight,
  FileVideo,
  File as FileIcon,
  Folder,
  FolderOpen,
  Search,
  HardDrive,
  Languages,
  Send,
  X,
} from "lucide-react";
import {
  api,
  PROVIDERS,
  type LibraryEntry,
  type RootInfo,
} from "@/lib/api";
import { LangField, ModelField } from "./Media";
import { cn } from "@/lib/utils";

function formatSize(bytes?: number): string {
  if (bytes === undefined) return "";
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

function formatDuration(s?: number | null): string {
  if (s === null || s === undefined) return "—";
  const total = Math.round(s);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

function FileIconFor({ entry }: { entry: LibraryEntry }) {
  if (entry.is_dir)
    return <Folder size={16} className="shrink-0 text-accent" />;
  if (entry.is_video)
    return <FileVideo size={16} className="shrink-0 text-success" />;
  return <FileIcon size={16} className="shrink-0 text-muted" />;
}

function VideoThumb({ entry }: { entry: LibraryEntry }) {
  const [failed, setFailed] = useState(false);
  if (!entry.is_video || !entry.thumbnail_url || failed) {
    return <FileIconFor entry={entry} />;
  }
  return (
    <img
      src={entry.thumbnail_url}
      alt=""
      loading="lazy"
      onError={() => setFailed(true)}
      className="h-9 w-16 shrink-0 rounded object-cover bg-surface-2 ring-1 ring-border"
    />
  );
}

function MetaBadges({ entry }: { entry: LibraryEntry }) {
  const badges: string[] = [];
  if (entry.duration_s !== undefined && entry.duration_s !== null)
    badges.push(formatDuration(entry.duration_s));
  if (entry.container) badges.push(entry.container);
  if (entry.audio_count !== undefined && entry.audio_count !== null)
    badges.push(`${entry.audio_count} audio`);
  if (entry.subtitle_count !== undefined && entry.subtitle_count !== null)
    badges.push(`${entry.subtitle_count} sub`);
  if (badges.length === 0) return null;
  return (
    <span className="ml-2 hidden items-center gap-1 lg:inline-flex">
      {badges.map((b) => (
        <span
          key={b}
          className="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-muted"
        >
          {b}
        </span>
      ))}
    </span>
  );
}

export default function Library() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const root = Number(params.get("root") ?? 0);
  const path = params.get("path") ?? "";
  const q = params.get("q") ?? "";
  const showAll = params.get("show_all") === "1";

  function update(patch: Record<string, string | null>) {
    const next = new URLSearchParams(params);
    for (const [k, v] of Object.entries(patch)) {
      if (v === null || v === "") next.delete(k);
      else next.set(k, v);
    }
    setParams(next, { replace: false });
  }

  const rootsQuery = useQuery({
    queryKey: ["library", "roots"],
    queryFn: () =>
      api.get<{ roots: RootInfo[] }>("/api/library/roots").then((r) => r.roots),
  });
  const roots = rootsQuery.data ?? [];

  const searching = q.trim().length > 0;

  const treeQuery = useQuery({
    queryKey: ["library", "tree", root, path, showAll],
    queryFn: () =>
      api.get<{
        folders: LibraryEntry[];
        files: LibraryEntry[];
      }>(" /api/library/tree".trim(), {
        root,
        path,
        show_all: showAll,
      }),
    enabled: !searching && roots.length > 0,
  });

  const searchQuery = useQuery({
    queryKey: ["library", "search", root, q, showAll],
    queryFn: () =>
      api.get<{ results: LibraryEntry[] }>("/api/library/search", {
        root,
        q,
        show_all: showAll,
      }),
    enabled: searching && roots.length > 0,
  });

  const crumbs = useMemo(() => {
    const parts = path.split("/").filter(Boolean);
    const acc: { label: string; path: string }[] = [];
    let cur = "";
    for (const p of parts) {
      cur = cur ? `${cur}/${p}` : p;
      acc.push({ label: p, path: cur });
    }
    return acc;
  }, [path]);

  const activeRoot = roots.find((r) => r.index === root);

  // FR-14 batch selection (current view only).
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  useEffect(() => {
    setSelected({});
  }, [root, path]);

  const currentEntries = useMemo(() => {
    if (searching) return searchQuery.data?.results ?? [];
    return treeQuery.data?.files ?? [];
  }, [searching, searchQuery.data, treeQuery.data]);

  const selectedEntries = useMemo(
    () => currentEntries.filter((e) => e.is_video && selected[e.rel_path]),
    [currentEntries, selected],
  );
  const selectedCount = selectedEntries.length;

  function toggle(rel: string) {
    setSelected((prev) => {
      const next = { ...prev };
      if (next[rel]) delete next[rel];
      else next[rel] = true;
      return next;
    });
  }

  function clearSelection() {
    setSelected({});
  }

  function setAllSelected(select: boolean) {
    setSelected((prev) => {
      const next = { ...prev };
      for (const entry of currentEntries) {
        if (!entry.is_video) continue;
        if (select) next[entry.rel_path] = true;
        else delete next[entry.rel_path];
      }
      return next;
    });
  }

  // Batch translate dialog state.
  const [batchOpen, setBatchOpen] = useState(false);
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("");
  const [targetLang, setTargetLang] = useState("");
  const [description, setDescription] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [batchError, setBatchError] = useState<string | null>(null);

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.getSettings(),
    staleTime: 5 * 60 * 1000,
  });
  const modelsQuery = useQuery({
    queryKey: ["models", provider],
    queryFn: () => api.listModels(provider),
    enabled: batchOpen,
    staleTime: 5 * 60 * 1000,
  });

  function openBatch() {
    const s = settingsQuery.data;
    if (s?.default_provider && PROVIDERS.some((p) => p.id === s.default_provider))
      setProvider(s.default_provider);
    else setProvider("openai");
    setModel(s?.default_model ?? "");
    setTargetLang(s?.default_target_language ?? "");
    setDescription("");
    setBatchError(null);
    setBatchOpen(true);
  }

  async function submitBatch() {
    if (selectedEntries.length === 0) return;
    setSubmitting(true);
    setBatchError(null);
    try {
      const bodies = selectedEntries.map((entry) => ({
        root,
        path: entry.rel_path,
        provider,
        model: model.trim() || undefined,
        target_language: targetLang.trim() || undefined,
        description: description.trim() || undefined,
        start_now: false,
      }));
      const results = await Promise.allSettled(
        bodies.map((body) => api.createJob(body)),
      );
      const failed = results.filter((r) => r.status === "rejected").length;
      if (failed === 0) {
        await api
          .putSettings({
            default_provider: provider,
            default_model: model.trim(),
            default_target_language: targetLang.trim(),
          })
          .catch(() => {});
        setBatchOpen(false);
        clearSelection();
        navigate("/jobs");
      } else {
        const firstError = results.find(
          (r) => r.status === "rejected",
        ) as PromiseRejectedResult | undefined;
        const msg =
          firstError?.reason instanceof Error
            ? firstError.reason.message
            : "failed";
        setBatchError(`${failed} of ${bodies.length} jobs failed to start: ${msg}`);
      }
    } catch (e) {
      setBatchError(e instanceof Error ? e.message : "batch enqueue failed");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">Library</h1>
        <div className="flex flex-wrap items-center gap-2">
          {roots.length > 1 && (
            <select
              value={root}
              onChange={(e) => update({ root: e.target.value, path: null })}
              className="rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-sm outline-none focus:border-accent"
            >
              {roots.map((r) => (
                <option key={r.index} value={r.index}>
                  {r.name}
                </option>
              ))}
            </select>
          )}
          <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-sm">
            <input
              type="checkbox"
              checked={showAll}
              onChange={(e) => update({ show_all: e.target.checked ? "1" : null })}
              className="accent-[var(--color-accent)]"
            />
            Show all
          </label>
          {selectedCount > 0 && (
            <>
              <button
                onClick={openBatch}
                className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-foreground hover:bg-accent-2"
              >
                <Languages size={14} />
                Translate ({selectedCount})
              </button>
              <button
                onClick={clearSelection}
                className="rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-sm hover:border-accent"
              >
                Clear
              </button>
            </>
          )}
        </div>
      </div>

      {/* Search box */}
      <div className="relative mb-4">
        <Search
          size={16}
          className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted"
        />
        <input
          value={q}
          onChange={(e) => update({ q: e.target.value || null })}
          placeholder="Search files by name…"
          className="w-full rounded-lg border border-border bg-surface-2 py-2 pl-9 pr-3 text-sm outline-none focus:border-accent"
        />
      </div>

      {rootsQuery.isLoading && (
        <p className="text-sm text-muted">Loading library…</p>
      )}

      {roots.length === 0 && !rootsQuery.isLoading && (
        <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-border bg-surface py-20 text-center">
          <HardDrive size={32} className="mb-3 text-muted" />
          <p className="text-sm font-medium">No media roots configured</p>
          <p className="mt-1 max-w-sm text-xs text-muted">
            Set <code>MEDIA_ROOTS</code> (or mount a media volume) and restart to
            browse your library.
          </p>
        </div>
      )}

      {roots.length > 0 && (
        <>
          {/* Breadcrumb (hidden while searching) */}
          {!searching && (
            <nav className="mb-3 flex flex-wrap items-center gap-1 text-sm">
              <button
                onClick={() => update({ path: null })}
                className={cn(
                  "flex items-center gap-1 rounded px-2 py-1 hover:bg-surface-2",
                  path === "" ? "text-foreground" : "text-muted",
                )}
              >
                <FolderOpen size={14} />
                {activeRoot?.name ?? `root ${root}`}
              </button>
              {crumbs.map((c) => (
                <span key={c.path} className="flex items-center gap-1">
                  <ChevronRight size={14} className="text-muted" />
                  <button
                    onClick={() => update({ path: c.path })}
                    className={cn(
                      "rounded px-2 py-1 hover:bg-surface-2",
                      c.path === path
                        ? "text-foreground"
                        : "text-muted",
                    )}
                  >
                    {c.label}
                  </button>
                </span>
              ))}
            </nav>
          )}

          {searching ? (
            <SearchResults
              loading={searchQuery.isPending}
              error={searchQuery.error?.message}
              results={searchQuery.data?.results ?? []}
              selected={selected}
              onToggle={toggle}
              onSelectAll={setAllSelected}
              onOpen={(rel) =>
                navigate(
                  `/media?root=${root}&path=${encodeURIComponent(rel)}`,
                )
              }
            />
          ) : (
            <TreeTable
              loading={treeQuery.isPending}
              error={treeQuery.error?.message}
              folders={treeQuery.data?.folders ?? []}
              files={treeQuery.data?.files ?? []}
              selected={selected}
              onToggle={toggle}
              onSelectAll={setAllSelected}
              onFolder={(rel) => update({ path: rel })}
              onFile={(rel) =>
                navigate(
                  `/media?root=${root}&path=${encodeURIComponent(rel)}`,
                )
              }
            />
          )}
        </>
      )}

      {batchOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/60 p-4">
          <div className="w-full max-w-lg rounded-xl border border-border bg-surface p-5 shadow-lg">
            <div className="mb-4 flex items-center justify-between">
              <h2 className="text-lg font-semibold">
                Translate {selectedCount} file{selectedCount === 1 ? "" : "s"}
              </h2>
              <button
                onClick={() => setBatchOpen(false)}
                className="text-muted hover:text-foreground"
              >
                <X size={18} />
              </button>
            </div>

            <div className="grid gap-4 sm:grid-cols-2">
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
              <div className="sm:col-span-2">
                <LangField value={targetLang} onChange={setTargetLang} />
              </div>
              <label className="flex flex-col gap-1 text-sm sm:col-span-2">
                <span className="text-muted">
                  Additional information (optional)
                </span>
                <textarea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Extra context applied to every selected episode"
                  rows={3}
                  className="resize-y rounded-lg border border-border bg-surface-2 px-3 py-2 outline-none focus:border-accent"
                />
              </label>
            </div>

            {PROVIDERS.find((p) => p.id === provider)?.needsKey && (
              <p className="mt-3 text-xs text-warning">
                This provider needs an API key — set it in Settings first.
              </p>
            )}

            {batchError && (
              <p className="mt-3 text-xs text-danger">{batchError}</p>
            )}

            <div className="mt-5 flex items-center justify-end gap-2">
              <button
                onClick={() => setBatchOpen(false)}
                className="rounded-lg border border-border bg-surface-2 px-4 py-2 text-sm hover:border-accent"
              >
                Cancel
              </button>
              <button
                onClick={submitBatch}
                disabled={submitting || selectedCount === 0}
                className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-2 disabled:opacity-50"
              >
                <Send size={14} />
                {submitting ? "Queuing…" : `Queue ${selectedCount} jobs`}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function TreeTable({
  loading,
  error,
  folders,
  files,
  selected,
  onToggle,
  onSelectAll,
  onFolder,
  onFile,
}: {
  loading: boolean;
  error?: string;
  folders: LibraryEntry[];
  files: LibraryEntry[];
  selected: Record<string, boolean>;
  onToggle: (rel: string) => void;
  onSelectAll: (select: boolean) => void;
  onFolder: (rel: string) => void;
  onFile: (rel: string) => void;
}) {
  if (loading) return <p className="text-sm text-muted">Loading…</p>;
  if (error)
    return (
      <div className="rounded-lg border border-danger/40 bg-danger/10 p-4 text-sm text-danger">
        {error}
      </div>
    );
  if (folders.length === 0 && files.length === 0)
    return (
      <p className="rounded-lg border border-border bg-surface p-6 text-center text-sm text-muted">
        This folder is empty.
      </p>
    );

  const selectableFiles = files.filter((f) => f.is_video);
  const allSelected =
    selectableFiles.length > 0 &&
    selectableFiles.every((f) => selected[f.rel_path]);
  const someSelected = selectableFiles.some((f) => selected[f.rel_path]);

  return (
    <div className="overflow-hidden rounded-xl border border-border bg-surface">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs uppercase tracking-wide text-muted">
            <th className="w-10 px-4 py-2">
              {selectableFiles.length > 1 && (
                <input
                  type="checkbox"
                  aria-label="Select all files in this folder"
                  title="Select all files in this folder"
                  checked={allSelected}
                  ref={(el) => {
                    if (el) el.indeterminate = someSelected && !allSelected;
                  }}
                  onChange={(e) => onSelectAll(e.target.checked)}
                  onClick={(e) => e.stopPropagation()}
                  className="accent-[var(--color-accent)]"
                />
              )}
            </th>
            <th className="px-4 py-2 font-medium">Name</th>
            <th className="px-4 py-2 text-right font-medium">Size</th>
          </tr>
        </thead>
        <tbody>
          {folders.map((f) => (
            <tr
              key={f.rel_path}
              onClick={() => onFolder(f.rel_path)}
              className="cursor-pointer border-b border-border/50 last:border-0 hover:bg-surface-2"
            >
              <td className="px-4 py-2" />
              <td className="flex items-center gap-2 px-4 py-2">
                <VideoThumb entry={f} />
                {f.name}
              </td>
              <td className="px-4 py-2 text-right text-muted">—</td>
            </tr>
          ))}
          {files.map((f) => (
            <tr
              key={f.rel_path}
              onClick={() => f.is_video && onFile(f.rel_path)}
              className={cn(
                "border-b border-border/50 last:border-0",
                f.is_video
                  ? "cursor-pointer hover:bg-surface-2"
                  : "opacity-70",
              )}
            >
              <td className="px-4 py-2">
                {f.is_video && (
                  <input
                    type="checkbox"
                    checked={!!selected[f.rel_path]}
                    onChange={() => onToggle(f.rel_path)}
                    onClick={(e) => e.stopPropagation()}
                    aria-label={`Select ${f.name}`}
                    className="accent-[var(--color-accent)]"
                  />
                )}
              </td>
              <td className="flex items-center gap-2 px-4 py-2">
                <VideoThumb entry={f} />
                {f.name}
                <MetaBadges entry={f} />
              </td>
              <td className="px-4 py-2 text-right text-muted">
                {formatSize(f.size)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SearchResults({
  loading,
  error,
  results,
  selected,
  onToggle,
  onSelectAll,
  onOpen,
}: {
  loading: boolean;
  error?: string;
  results: LibraryEntry[];
  selected: Record<string, boolean>;
  onToggle: (rel: string) => void;
  onSelectAll: (select: boolean) => void;
  onOpen: (rel: string) => void;
}) {
  if (loading) return <p className="text-sm text-muted">Searching…</p>;
  if (error)
    return (
      <div className="rounded-lg border border-danger/40 bg-danger/10 p-4 text-sm text-danger">
        {error}
      </div>
    );
  if (results.length === 0)
    return (
      <p className="rounded-lg border border-border bg-surface p-6 text-center text-sm text-muted">
        No files match your search.
      </p>
    );

  const selectableResults = results.filter((f) => f.is_video);
  const allSelected =
    selectableResults.length > 0 &&
    selectableResults.every((f) => selected[f.rel_path]);
  const someSelected = selectableResults.some((f) => selected[f.rel_path]);

  return (
    <div className="overflow-hidden rounded-xl border border-border bg-surface">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs uppercase tracking-wide text-muted">
            <th className="w-10 px-4 py-2">
              {selectableResults.length > 1 && (
                <input
                  type="checkbox"
                  aria-label="Select all search results"
                  title="Select all search results"
                  checked={allSelected}
                  ref={(el) => {
                    if (el) el.indeterminate = someSelected && !allSelected;
                  }}
                  onChange={(e) => onSelectAll(e.target.checked)}
                  onClick={(e) => e.stopPropagation()}
                  className="accent-[var(--color-accent)]"
                />
              )}
            </th>
            <th className="px-4 py-2 font-medium">Name</th>
            <th className="px-4 py-2 font-medium">Path</th>
            <th className="px-4 py-2 text-right font-medium">Size</th>
          </tr>
        </thead>
        <tbody>
          {results.map((f) => (
            <tr
              key={f.rel_path}
              onClick={() => f.is_video && onOpen(f.rel_path)}
              className={cn(
                "border-b border-border/50 last:border-0",
                f.is_video ? "cursor-pointer hover:bg-surface-2" : "opacity-70",
              )}
            >
              <td className="px-4 py-2">
                {f.is_video && (
                  <input
                    type="checkbox"
                    checked={!!selected[f.rel_path]}
                    onChange={() => onToggle(f.rel_path)}
                    onClick={(e) => e.stopPropagation()}
                    aria-label={`Select ${f.name}`}
                    className="accent-[var(--color-accent)]"
                  />
                )}
              </td>
              <td className="flex items-center gap-2 px-4 py-2">
                <VideoThumb entry={f} />
                {f.name}
                <MetaBadges entry={f} />
              </td>
              <td className="px-4 py-2 text-muted">{f.rel_path}</td>
              <td className="px-4 py-2 text-right text-muted">
                {formatSize(f.size)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
