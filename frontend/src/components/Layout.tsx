import { useState, type ReactNode } from "react";
import { NavLink } from "react-router-dom";
import {
  Captions,
  Clapperboard,
  Film,
  HeartPulse,
  Moon,
  SlidersHorizontal,
  Sun,
} from "lucide-react";
import { cn } from "@/lib/utils";

type Theme = "dark" | "light";

const nav = [
  { to: "/library", label: "Library", icon: Film },
  { to: "/jobs", label: "Jobs", icon: Captions },
  { to: "/settings", label: "Settings", icon: SlidersHorizontal },
  { to: "/health", label: "Health", icon: HeartPulse },
];

function getInitialTheme(): Theme {
  return document.documentElement.dataset.theme === "light" ? "light" : "dark";
}

export default function Layout({ children }: { children: ReactNode }) {
  const [theme, setTheme] = useState<Theme>(getInitialTheme);

  function toggleTheme() {
    const next: Theme = theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("transflator-theme", next);
    setTheme(next);
  }

  return (
    <div className="flex h-full flex-col md:flex-row">
      <aside className="flex shrink-0 flex-col border-b border-border bg-surface md:w-56 md:border-b-0 md:border-r">
        <div className="flex items-center gap-2 px-4 py-3 md:py-4">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent/20 text-accent">
            <Clapperboard size={18} />
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
        <div className="px-2 pb-2 md:pb-0">
          <button
            type="button"
            onClick={toggleTheme}
            className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-muted transition-colors hover:bg-surface-2 hover:text-foreground"
          >
            {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
            {theme === "dark" ? "Light mode" : "Dark mode"}
          </button>
        </div>
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
