import { useEffect, useState, type ReactNode } from "react";
import { getToken, setToken } from "@/lib/auth";

// D7 auth gate: when the backend returns 401 (AUTH_TOKEN set and missing/wrong
// token), prompt for the shared access token. When the backend is open (no
// AUTH_TOKEN), this never triggers.
export default function AuthGate({ children }: { children: ReactNode }) {
  const [locked, setLocked] = useState(false);
  const [value, setValue] = useState("");

  useEffect(() => {
    const onUnauthorized = () => setLocked(true);
    window.addEventListener("transflator:unauthorized", onUnauthorized);
    return () =>
      window.removeEventListener("transflator:unauthorized", onUnauthorized);
  }, []);

  if (!locked) return <>{children}</>;

  return (
    <div className="flex h-full items-center justify-center bg-background">
      <form
        className="w-80 rounded-xl border border-border bg-surface p-6"
        onSubmit={(e) => {
          e.preventDefault();
          setToken(value.trim() || null);
          setLocked(false);
          window.location.reload();
        }}
      >
        <h1 className="mb-1 text-lg font-semibold">Access token required</h1>
        <p className="mb-4 text-sm text-muted">
          This instance is protected. Enter the shared access token.
        </p>
        <input
          type="password"
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="Access token"
          className="w-full rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
        />
        <div className="mt-4 flex gap-2">
          <button
            type="submit"
            className="flex-1 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-white hover:bg-accent-2"
          >
            Unlock
          </button>
          <button
            type="button"
            onClick={() => {
              setToken(null);
              setLocked(false);
            }}
            className="rounded-lg border border-border px-3 py-2 text-sm text-muted hover:text-foreground"
          >
            Cancel
          </button>
        </div>
        {getToken() && (
          <p className="mt-3 text-xs text-danger">
            The saved token was rejected.
          </p>
        )}
      </form>
    </div>
  );
}
