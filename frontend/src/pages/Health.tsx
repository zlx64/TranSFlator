import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, Download, FileText, RefreshCw, XCircle } from "lucide-react";
import { useState } from "react";
import { api, type HealthResponse, type LogFile } from "@/lib/api";

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  const digits = value >= 10 || index === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[index]}`;
}

function formatTime(iso: string | null): string {
  if (!iso) return "unknown";
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
}

export default function Health() {
  const health = useQuery({
    queryKey: ["health"],
    queryFn: () => api.get<HealthResponse>("/api/healthz"),
    refetchInterval: 10_000,
  });

  const logs = useQuery({
    queryKey: ["health", "logs"],
    queryFn: () => api.listLogs(),
    refetchInterval: 10_000,
  });

  const [downloading, setDownloading] = useState<string | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);

  async function handleDownload(file: LogFile) {
    setDownloading(file.name);
    setDownloadError(null);
    try {
      await api.downloadLog(file.name);
    } catch (error) {
      setDownloadError(error instanceof Error ? error.message : "download failed");
    } finally {
      setDownloading(null);
    }
  }

  return (
    <div>
      <h1 className="mb-4 text-xl font-semibold">Health</h1>
      <div className="rounded-xl border border-border bg-surface p-6">
        {health.isPending && <p className="text-sm text-muted">Checking…</p>}
        {health.isError && (
          <div className="flex items-center gap-3 text-danger">
            <XCircle size={20} />
            <div>
              <p className="text-sm font-medium">Backend unreachable</p>
              <p className="text-xs text-muted">
                {health.error instanceof Error ? health.error.message : "unknown error"}
              </p>
            </div>
          </div>
        )}
        {health.data && (
          <div className="flex items-center gap-3 text-success">
            <CheckCircle2 size={20} />
            <div>
              <p className="text-sm font-medium">
                {health.data.service} is healthy ({health.data.status})
              </p>
              <p className="text-xs text-muted">server time {health.data.time}</p>
            </div>
          </div>
        )}
      </div>

      <div className="mt-4 rounded-xl border border-border bg-surface p-6">
        <div className="mb-4 flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <FileText size={18} />
            <h2 className="text-sm font-semibold">Logs</h2>
          </div>
          <button
            type="button"
            onClick={() => logs.refetch()}
            className="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs font-medium hover:bg-background disabled:opacity-50"
            disabled={logs.isFetching}
          >
            <RefreshCw size={14} className={logs.isFetching ? "animate-spin" : ""} />
            Refresh
          </button>
        </div>

        {logs.isPending && <p className="text-sm text-muted">Loading logs…</p>}
        {logs.isError && (
          <p className="text-sm text-danger">
            {logs.error instanceof Error ? logs.error.message : "Could not load logs."}
          </p>
        )}
        {logs.data && logs.data.logs.length === 0 && (
          <p className="text-sm text-muted">No log files yet.</p>
        )}
        {logs.data && logs.data.logs.length > 0 && (
          <ul className="space-y-2">
            {logs.data.logs.map((file) => (
              <li
                key={file.name}
                className="flex items-center justify-between gap-3 rounded-lg border border-border bg-background px-3 py-2"
              >
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{file.name}</p>
                  <p className="text-xs text-muted">
                    {formatBytes(file.size)} · {formatTime(file.modified_at)}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => handleDownload(file)}
                  disabled={downloading !== null}
                  className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs font-medium hover:bg-surface disabled:opacity-50"
                >
                  <Download size={14} className={downloading === file.name ? "animate-pulse" : ""} />
                  {downloading === file.name ? "Downloading…" : "Download"}
                </button>
              </li>
            ))}
          </ul>
        )}

        {downloadError && <p className="mt-3 text-xs text-danger">{downloadError}</p>}
      </div>
    </div>
  );
}