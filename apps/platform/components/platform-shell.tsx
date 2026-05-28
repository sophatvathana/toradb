"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Activity,
  BarChart3,
  CircleDot,
  Clock3,
  Database,
  Gauge,
  LayoutGrid,
  Layers,
  ServerCog,
  Table2,
  Upload,
  Workflow,
} from "lucide-react";
import type { ComponentType, ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { cacheHitRatio } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

const ICON_RAIL_WIDTH = 68;
const NAV_SIDEBAR_WIDTH = 240;

const NAV_ITEMS = [
  { href: "/overview", icon: Gauge, title: "Workspace Overview" },
  { href: "/query", icon: Database, title: "Query" },
  { href: "/query-log", icon: Clock3, title: "Query Log" },
  { href: "/catalog", icon: Table2, title: "Catalog" },
  { href: "/schema", icon: ServerCog, title: "Schema" },
  { href: "/views", icon: Layers, title: "Materialized Views" },
  { href: "/ingest", icon: Upload, title: "Ingest" },
  { href: "/jobs", icon: Activity, title: "Background Jobs" },
] as const;

const RAIL_ITEMS = [
  { href: "/overview", icon: LayoutGrid, label: "Overview" },
  { href: "/query", icon: Database, label: "Query" },
  { href: "/ingest", icon: Upload, label: "Ingest" },
  { href: "/jobs", icon: Workflow, label: "Jobs" },
  { href: "/query-log", icon: BarChart3, label: "Log" },
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
  const sidebarOffset = ICON_RAIL_WIDTH + NAV_SIDEBAR_WIDTH;

  return (
    <div className="min-h-dvh bg-background text-foreground">
      {/* Icon rail — fixed to viewport */}
      <aside
        className="fixed inset-y-0 left-0 z-50 flex w-[68px] flex-col border-r border-border bg-card/80 p-3 backdrop-blur-md"
        aria-label="Quick navigation"
      >
        <div className="mb-2 shrink-0 rounded-lg border border-border bg-card p-2">
          <ServerCog className="size-5 text-primary" />
        </div>
        <nav className="flex flex-1 flex-col items-center gap-2 overflow-y-auto">
          {RAIL_ITEMS.map(({ href, icon: Icon, label }) => (
            <Link
              key={href}
              href={href}
              title={label}
              aria-label={label}
              className={`shrink-0 rounded-md border p-2 transition-colors ${
                isNavActive(pathname, href)
                  ? "border-primary/50 bg-primary/20 text-primary"
                  : "border-border bg-card text-muted-foreground hover:border-primary/30 hover:text-foreground"
              }`}
            >
              <Icon className="size-4" />
            </Link>
          ))}
        </nav>
      </aside>

      {/* Primary sidebar — fixed, scrollable nav when content is tall */}
      <aside
        className="fixed inset-y-0 left-[68px] z-40 flex w-60 flex-col border-r border-border bg-card/90 backdrop-blur-md"
        aria-label="Platform navigation"
      >
        <div className="shrink-0 border-b border-border/60 px-4 py-4">
          <h1 className="text-lg font-semibold">ToraDB Console</h1>
          <p
            className="mt-0.5 truncate text-xs text-muted-foreground"
            title={health?.db_path}
          >
            {health?.db_path ?? "Connecting…"}
          </p>
        </div>

        <nav className="min-h-0 flex-1 space-y-1 overflow-y-auto px-3 py-3 text-sm">
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

        <div className="shrink-0 border-t border-border/60 p-4">
          <div className="rounded-lg border border-border bg-muted/30 p-3">
            <div className="text-xs uppercase tracking-wide text-muted-foreground">
              Cluster Health
            </div>
            <div className="mt-2 flex items-center gap-2">
              <CircleDot
                className={`size-3 shrink-0 ${health?.status === "ok" ? "text-emerald-400" : "text-amber-400"}`}
              />
              <span className="text-sm">
                {health?.status === "ok" ? "Operational" : "Unknown"}
              </span>
            </div>
            <div className="mt-2 text-xs text-muted-foreground">
              {health ? `${health.tables.length} tables` : "—"}
            </div>
          </div>
        </div>
      </aside>

      {/* Main content — offset by fixed sidebars */}
      <div
        className="min-h-dvh min-w-0"
        style={{ paddingLeft: sidebarOffset }}
      >
        <main className="min-h-dvh p-5">
          <header className="sticky top-0 z-20 mb-4 flex items-center justify-between rounded-xl border border-border bg-card/80 px-4 py-3 shadow-sm backdrop-blur-md">
            <div>
              <h2 className="text-xl font-semibold">Operational Control Plane</h2>
              <p className="text-sm text-muted-foreground">
                Live metrics from toradb-api
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Badge variant="outline">cache hit {ratio}</Badge>
              <Badge variant="secondary">{metrics?.query_count ?? 0} queries</Badge>
            </div>
          </header>

          {error && (
            <div className="mb-4 rounded-md border border-destructive/50 bg-destructive/20 p-2 text-sm text-destructive-foreground">
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
      className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 transition-colors ${
        selected
          ? "bg-primary/20 text-primary"
          : "text-muted-foreground hover:bg-muted hover:text-foreground"
      }`}
    >
      <Icon className="size-4 shrink-0" />
      <span className="truncate">{title}</span>
    </Link>
  );
}
