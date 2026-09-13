import type { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import { Film, ListChecks, Settings as SettingsIcon, Activity } from "lucide-react";
import { cn } from "@/lib/utils";

const nav = [
  { to: "/library", label: "Library", icon: Film },
  { to: "/jobs", label: "Jobs", icon: ListChecks },
  { to: "/settings", label: "Settings", icon: SettingsIcon },
  { to: "/health", label: "Health", icon: Activity },
];

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full flex-col md:flex-row">
      <aside className="flex shrink-0 flex-col border-b border-border bg-surface md:w-56 md:border-b-0 md:border-r">
        <div className="flex items-center gap-2 px-4 py-3 md:py-4">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent/20 text-accent">
            <Film size={18} />
          </div>
          <span className="text-sm font-semibold tracking-tight">
            TranSFlator
          </span>
        </div>
        <nav className="flex gap-1 overflow-x-auto px-2 py-2 md:flex-1 md:flex-col md:overflow-visible">
          {nav.map(({ to, label, icon: Icon }) => (
            <NavLink
              key={to}
              to={to}
              className={({ isActive }) =>
                cn(
                  "flex shrink-0 items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors",
                  isActive
                    ? "bg-accent/15 text-accent"
                    : "text-muted hover:bg-surface-2 hover:text-foreground",
                )
              }
            >
              <Icon size={16} />
              {label}
            </NavLink>
          ))}
        </nav>
        <div className="hidden px-4 py-3 text-xs text-muted md:block">
          self-hosted
        </div>
      </aside>
      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl px-4 py-4 md:px-6 md:py-6">
          {children}
        </div>
      </main>
    </div>
  );
}
