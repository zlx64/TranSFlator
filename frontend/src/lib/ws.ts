import { useEffect, useRef, useState } from "react";
import { getToken } from "./auth";
import type { JobEvent } from "./api";

export type WsStatus = "connecting" | "open" | "closed";

interface UseJobEventsOptions {
  onEvent: (event: JobEvent) => void;
  enabled?: boolean;
}

function buildUrl(jobId: string): string {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const token = getToken();
  const qs = token ? `?token=${encodeURIComponent(token)}` : "";
  return `${proto}//${window.location.host}/api/jobs/${jobId}/events${qs}`;
}

// Live job event stream (FR-16). Reconnects with backoff on drop. Terminal
// states (done/failed/canceled) stop reconnecting.
export function useJobEvents(
  jobId: string | null,
  { onEvent, enabled = true }: UseJobEventsOptions,
) {
  const [status, setStatus] = useState<WsStatus>("closed");
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  useEffect(() => {
    if (!jobId || !enabled) return;

    let ws: WebSocket | null = null;
    let closed = false;
    let terminal = false;
    let retries = 0;
    let timer: number | undefined;

    const connect = () => {
      if (closed) return;
      setStatus("connecting");
      ws = new WebSocket(buildUrl(jobId));

      ws.onopen = () => {
        retries = 0;
        setStatus("open");
      };

      ws.onmessage = (msg) => {
        try {
          const event = JSON.parse(msg.data) as JobEvent;
          if (
            event.type === "done" ||
            event.type === "failed" ||
            (event.type === "status" &&
              (event.status === "done" ||
                event.status === "failed" ||
                event.status === "canceled"))
          ) {
            terminal = true;
          }
          onEventRef.current(event);
        } catch {
          // ignore malformed frames
        }
      };

      ws.onclose = () => {
        setStatus("closed");
        if (closed || terminal) return;
        retries += 1;
        const delay = Math.min(1000 * 2 ** retries, 15000);
        timer = window.setTimeout(connect, delay);
      };

      ws.onerror = () => {
        ws?.close();
      };
    };

    connect();

    return () => {
      closed = true;
      if (timer !== undefined) window.clearTimeout(timer);
      if (ws) {
        ws.onclose = null;
        ws.close();
      }
    };
  }, [jobId, enabled]);

  return status;
}
