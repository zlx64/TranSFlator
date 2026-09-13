import { useMemo } from "react";
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
} from "lucide-react";
import {
  api,
  type LibraryEntry,
  type RootInfo,
} from "@/lib/api";
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

function FileIconFor({ entry }: { entry: LibraryEntry }) {
  if (entry.is_dir)
    return <Folder size={16} className="shrink-0 text-accent" />;
  if (entry.is_video)
    return <FileVideo size={16} className="shrink-0 text-success" />;
  return <FileIcon size={16} className="shrink-0 text-muted" />;
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

  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">Library</h1>
        <div className="flex items-center gap-2">
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
    </div>
  );
}

function TreeTable({
  loading,
  error,
  folders,
  files,
  onFolder,
  onFile,
}: {
  loading: boolean;
  error?: string;
  folders: LibraryEntry[];
  files: LibraryEntry[];
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

  return (
    <div className="overflow-hidden rounded-xl border border-border bg-surface">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs uppercase tracking-wide text-muted">
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
              <td className="flex items-center gap-2 px-4 py-2">
                <FileIconFor entry={f} />
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
              <td className="flex items-center gap-2 px-4 py-2">
                <FileIconFor entry={f} />
                {f.name}
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
  onOpen,
}: {
  loading: boolean;
  error?: string;
  results: LibraryEntry[];
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
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-surface">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs uppercase tracking-wide text-muted">
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
              <td className="flex items-center gap-2 px-4 py-2">
                <FileIconFor entry={f} />
                {f.name}
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
