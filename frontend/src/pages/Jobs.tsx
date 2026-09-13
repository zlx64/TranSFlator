import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronRight,
  Download,
  ListChecks,
  RefreshCw,
  RotateCw,
  Terminal,
  XCircle,
} from "lucide-react";
import {
  api,
  type Job,
  type JobEvent,
  type JobStatus,
} from "@/lib/api";
import { useJobEvents } from "@/lib/ws";
import { cn } from "@/lib/utils";

const STATUS_STYLES: Record<JobStatus, string> = {
  queued: "bg-muted/15 text-muted",
  running: "bg-accent/15 text-accent",
  done: "bg-success/15 text-success",
  failed: "bg-danger/15 text-danger",
  canceled: "bg-warning/15 text-warning",
  interrupted: "bg-warning/15 text-warning",
};

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

function StatusChip({ status }: { status: JobStatus }) {
  return (
    <span
      className={cn(
        "shrink-0 rounded-full px-2 py-0.5 text-xs font-medium capitalize",
        STATUS_STYLES[status],
      )}
    >
      {status}
    </span>
  );
}

function ProgressBar({ pct }: { pct: number }) {
  const clamped = Math.min(100, Math.max(0, pct));
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
      <div
        className="h-full rounded-full bg-accent transition-all"
        style={{ width: `${clamped}%` }}
      />
    </div>
  );
}

export default function Jobs() {
  const queryClient = useQueryClient();
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [retryMode, setRetryMode] = useState<Record<string, string>>({});
  const [flash, setFlash] = useState<string | null>(null);

  const jobsQuery = useQuery({
    queryKey: ["jobs", "list"],
    queryFn: () => api.listJobs(50).then((r) => r.jobs),
    refetchInterval: 3000,
  });
  const jobs = jobsQuery.data ?? [];

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["jobs", "list"] });

  // Live log + progress for the expanded job, streamed over the per-job WS.
  const [liveLog, setLiveLog] = useState<string[]>([]);
  const [livePct, setLivePct] = useState<number | null>(null);
  const seededFor = useRef<string | null>(null);

  // Seed the expanded job's log once from the persisted tail (then the WS
  // appends live lines). Waits until the job is present in the list.
  useEffect(() => {
    if (expandedId === null) {
      setLiveLog([]);
      setLivePct(null);
      seededFor.current = null;
      return;
    }
    if (seededFor.current === expandedId) return;
    const job = jobsQuery.data?.find((j) => j.id === expandedId);
    if (!job) return;
    setLiveLog(job.log_tail ? job.log_tail.split("\n").filter(Boolean) : []);
    setLivePct(job.progress_pct);
    seededFor.current = expandedId;
  }, [expandedId, jobsQuery.data]);

  const wsStatus = useJobEvents(expandedId, {
    onEvent: (ev: JobEvent) => {
      if (ev.type === "log") {
        setLiveLog((l) => [...l, ev.line].slice(-800));
      } else if (ev.type === "progress") {
        setLivePct(ev.pct);
      } else if (ev.type === "done" || ev.type === "failed") {
        invalidate();
      }
    },
  });

  function showError(e: unknown) {
    setFlash(e instanceof Error ? e.message : "something went wrong");
    window.setTimeout(() => setFlash(null), 5000);
  }

  async function onCancel(job: Job) {
    try {
      await api.cancelJob(job.id);
      invalidate();
    } catch (e) {
      showError(e);
    }
  }

  async function onRetry(job: Job) {
    const mode = retryMode[job.id] ?? "rerun";
    try {
      await api.retryJob(job.id, mode);
      invalidate();
    } catch (e) {
      showError(e);
    }
  }

  async function onDownload(job: Job) {
    if (!job.output_path) return;
    try {
      await api.downloadJob(job.id, basename(job.output_path));
    } catch (e) {
      showError(e);
    }
  }

  return (
    <div>
      <div className="mb-4 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Jobs</h1>
        <button
          onClick={() => jobsQuery.refetch()}
          disabled={jobsQuery.isRefetching}
          className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50"
        >
          <RefreshCw
            size={14}
            className={cn(jobsQuery.isRefetching && "animate-spin")}
          />
          Refresh
        </button>
      </div>

      {flash && (
        <div className="mb-4 rounded-lg border border-danger/40 bg-danger/10 p-3 text-sm text-danger">
          {flash}
        </div>
      )}

      {jobsQuery.isPending && (
        <p className="text-sm text-muted">Loading jobs…</p>
      )}

      {jobsQuery.error && (
        <div className="rounded-lg border border-danger/40 bg-danger/10 p-4 text-sm text-danger">
          {jobsQuery.error.message}
        </div>
      )}

      {jobs.length === 0 && !jobsQuery.isPending && (
        <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-border bg-surface py-20 text-center">
          <ListChecks size={32} className="mb-3 text-muted" />
          <p className="text-sm font-medium">No translation jobs yet</p>
          <p className="mt-1 max-w-sm text-xs text-muted">
            Pick a media file with a text subtitle track and start a translation.
          </p>
          <Link
            to="/library"
            className="mt-4 inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground hover:bg-accent-2"
          >
            Browse library
          </Link>
        </div>
      )}

      {jobs.length > 0 && (
        <div className="space-y-2">
          {jobs.map((job) => {
            const isExpanded = job.id === expandedId;
            const active =
              job.status === "queued" || job.status === "running";
            const terminal =
              job.status === "done" ||
              job.status === "failed" ||
              job.status === "canceled" ||
              job.status === "interrupted";
            const pct = isExpanded
              ? Math.max(job.progress_pct, livePct ?? 0)
              : job.progress_pct;

            return (
              <div
                key={job.id}
                className="overflow-hidden rounded-xl border border-border bg-surface"
              >
                <button
                  onClick={() => setExpandedId(isExpanded ? null : job.id)}
                  className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-surface-2/50"
                >
                  <span className="shrink-0 text-muted">
                    {isExpanded ? (
                      <ChevronDown size={16} />
                    ) : (
                      <ChevronRight size={16} />
                    )}
                  </span>
                  <StatusChip status={job.status} />
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {basename(job.source_path)}
                  </span>
                  <span className="hidden shrink-0 text-xs text-muted sm:inline">
                    → {job.target_language}
                  </span>
                  <span className="hidden shrink-0 text-xs text-muted md:inline">
                    {job.provider}
                  </span>
                  <span className="hidden w-40 shrink-0 items-center gap-2 sm:flex">
                    <ProgressBar pct={pct} />
                    <span className="w-9 text-right text-xs text-muted">
                      {pct}%
                    </span>
                  </span>
                </button>

                {isExpanded && (
                  <div className="border-t border-border px-4 py-3">
                    <div className="mb-3 flex flex-wrap items-center gap-3 text-sm">
                      {active && (
                        <button
                          onClick={() => onCancel(job)}
                          className="inline-flex items-center gap-1.5 rounded-lg border border-danger/40 bg-danger/10 px-3 py-1.5 text-sm text-danger hover:bg-danger/20"
                        >
                          <XCircle size={14} />
                          Cancel
                        </button>
                      )}
                      {terminal && (
                        <div className="flex items-center gap-2">
                          <select
                            value={retryMode[job.id] ?? "rerun"}
                            onChange={(e) =>
                              setRetryMode((m) => ({
                                ...m,
                                [job.id]: e.target.value,
                              }))
                            }
                            className="rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-sm outline-none focus:border-accent"
                          >
                            <option value="rerun">Re-run</option>
                            <option value="retranslate">Re-translate</option>
                            <option value="reparse">Re-parse</option>
                          </select>
                          <button
                            onClick={() => onRetry(job)}
                            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-foreground hover:bg-accent-2"
                          >
                            <RotateCw size={14} />
                            Retry
                          </button>
                        </div>
                      )}
                      {job.output_path && (
                        <button
                          onClick={() => onDownload(job)}
                          className="inline-flex items-center gap-1.5 rounded-lg border border-success/40 bg-success/10 px-3 py-1.5 text-sm text-success hover:bg-success/20"
                        >
                          <Download size={14} />
                          Download {basename(job.output_path)}
                        </button>
                      )}
                      {job.error_message && (
                        <span className="text-xs text-danger">
                          {job.error_message}
                        </span>
                      )}
                      <span className="ml-auto text-xs text-muted">
                        live: {wsStatus}
                      </span>
                    </div>

                    <div className="rounded-lg border border-border bg-background p-3">
                      <div className="mb-2 flex items-center gap-2 text-xs text-muted">
                        <Terminal size={13} />
                        Log
                      </div>
                      <pre className="max-h-64 overflow-auto whitespace-pre-wrap font-mono text-xs leading-relaxed">
                        {liveLog.length > 0
                          ? liveLog.join("\n")
                          : "(no log yet)"}
                      </pre>
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
