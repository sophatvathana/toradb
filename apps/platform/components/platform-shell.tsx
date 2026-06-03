"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Activity,
  BarChart3,
  Clock3,
  Database,
  Gauge,
  Layers,
  Plug,
  RefreshCw,
  ScrollText,
  MessageSquare,
  Search,
  ServerCog,
  Settings,
  Table2,
  Upload,
} from "lucide-react";
import type { ComponentType, ReactNode } from "react";

import { GlobalSearch } from "@/components/global-search";
import { Badge } from "@/components/ui/badge";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { cacheHitRatio } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

const SIDEBAR_WIDTH = 248;

const NAV_ITEMS = [
  { href: "/overview", icon: Gauge, title: "Overview" },
  { href: "/search", icon: Search, title: "Search" },
  { href: "/chat", icon: MessageSquare, title: "Chat" },
  { href: "/query", icon: Database, title: "Query" },
  { href: "/analytics", icon: BarChart3, title: "Analytics" },
  { href: "/query-log", icon: Clock3, title: "Query Log" },
  { href: "/provenance", icon: ScrollText, title: "Provenance" },
  { href: "/catalog", icon: Table2, title: "Catalog" },
  { href: "/schema", icon: ServerCog, title: "Schema" },
  { href: "/views", icon: Layers, title: "Materialized Views" },
  { href: "/ingest", icon: Upload, title: "Ingest" },
  { href: "/connections", icon: Plug, title: "Connections" },
  { href: "/sync", icon: RefreshCw, title: "Sync" },
  { href: "/jobs", icon: Activity, title: "Background Jobs" },
  { href: "/settings", icon: Settings, title: "Settings" },
] as const;

function isNavActive(pathname: string, href: string): boolean {
  if (pathname === href) return true;
  if (href !== "/overview" && pathname.startsWith(`${href}/`)) return true;
  if (href === "/catalog" && pathname.startsWith("/catalog/")) return true;
  return false;
}

export function PlatformShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const health = usePlatformStore((s) => s.health);
  const metrics = usePlatformStore((s) => s.metrics);
  const error = usePlatformStore((s) => s.error);

  const ratio = cacheHitRatio(metrics);

  return (
    <div className="min-h-dvh bg-background text-foreground">
      {/* Single calm sidebar */}
      <aside
        className="fixed inset-y-0 left-0 z-40 flex w-62 flex-col border-r border-border bg-card"
        style={{ width: SIDEBAR_WIDTH }}
        aria-label="Platform navigation"
      >
        <div className="flex shrink-0 items-center gap-2 px-4 py-4">
          <Database className="size-5 text-primary" />
          <div className="min-w-0">
            <div className="font-mono text-sm font-medium tracking-tight">ToraDB</div>
            <p
              className="truncate text-xs text-muted-foreground"
              title={health?.db_path}
            >
              {health?.db_path ?? "Connecting…"}
            </p>
          </div>
        </div>

        <nav className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-3 py-2 text-sm">
          {NAV_ITEMS.map(({ href, icon, title }) => (
            <NavRow
              key={href}
              href={href}
              icon={icon}
              title={title}
              selected={isNavActive(pathname, href)}
            />
          ))}
        </nav>

        <div className="shrink-0 border-t border-border p-4">
          <div className="rounded-lg border border-border bg-muted/40 p-3">
            <div className="text-xs uppercase tracking-wide text-muted-foreground">
              Cluster Health
            </div>
            <div className="mt-2 flex items-center gap-2 text-sm">
              <span
                className={`inline-block size-2 shrink-0 rounded-full ${
                  health?.status === "ok" ? "bg-success" : "bg-warning"
                }`}
              />
              {health?.status === "ok" ? "Operational" : "Unknown"}
            </div>
            <div className="mt-1 text-xs text-muted-foreground">
              {health ? `${health.tables.length} tables` : "—"}
            </div>
          </div>
        </div>
      </aside>

      {/* Main content — offset by the fixed sidebar */}
      <div className="min-h-dvh min-w-0" style={{ paddingLeft: SIDEBAR_WIDTH }}>
        <main className="min-h-dvh px-6 pb-8">
          <header className="sticky top-0 z-20 -mx-6 mb-6 flex items-center justify-between border-b border-border bg-background/80 px-6 py-3 backdrop-blur-sm">
            <h2 className="text-lg font-semibold tracking-tight">Control plane</h2>
            <div className="flex shrink-0 items-center gap-2">
              <ThemeToggle />
              <GlobalSearch />
              <Badge variant="outline">cache hit {ratio}</Badge>
              <Badge variant="secondary">{metrics?.query_count ?? 0} queries</Badge>
            </div>
          </header>

          {error && (
            <div className="mb-4 rounded-md border border-destructive/50 bg-destructive/15 p-2 text-sm text-destructive">
              {error}
            </div>
          )}

          {children}
        </main>
      </div>
    </div>
  );
}

function NavRow({
  href,
  icon: Icon,
  title,
  selected,
}: {
  href: string;
  icon: ComponentType<{ className?: string }>;
  title: string;
  selected: boolean;
}) {
  return (
    <Link
      href={href}
      className={`flex w-full items-center gap-2.5 rounded-md border-l-2 px-2.5 py-2 transition-colors ${
        selected
          ? "border-primary bg-accent font-medium text-foreground"
          : "border-transparent text-muted-foreground hover:bg-accent hover:text-foreground"
      }`}
    >
      <Icon className={`size-4 shrink-0 ${selected ? "text-primary" : ""}`} />
      <span className="truncate">{title}</span>
    </Link>
  );
}
