import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, XCircle } from "lucide-react";
import { api, type HealthResponse } from "@/lib/api";

export default function Health() {
  const { data, isError, isPending, error } = useQuery({
    queryKey: ["health"],
    queryFn: () => api.get<HealthResponse>("/api/healthz"),
    refetchInterval: 10_000,
  });

  return (
    <div>
      <h1 className="mb-4 text-xl font-semibold">Health</h1>
      <div className="rounded-xl border border-border bg-surface p-6">
        {isPending && <p className="text-sm text-muted">Checking…</p>}
        {isError && (
          <div className="flex items-center gap-3 text-danger">
            <XCircle size={20} />
            <div>
              <p className="text-sm font-medium">Backend unreachable</p>
              <p className="text-xs text-muted">
                {error instanceof Error ? error.message : "unknown error"}
              </p>
            </div>
          </div>
        )}
        {data && (
          <div className="flex items-center gap-3 text-success">
            <CheckCircle2 size={20} />
            <div>
              <p className="text-sm font-medium">
                {data.service} is healthy ({data.status})
              </p>
              <p className="text-xs text-muted">server time {data.time}</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
