"use client";

import Link from "next/link";
import { ArrowUpRight, Database, RefreshCw, Timer } from "lucide-react";
import { useMemo, type ComponentType } from "react";

import { IngestJobProgress } from "@/components/ingest-job-progress";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cacheHitRatio } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

export default function OverviewPage() {
  const metrics = usePlatformStore((s) => s.metrics);
  const tables = usePlatformStore((s) => s.tables);
  const jobs = usePlatformStore((s) => s.jobs);
  const history = usePlatformStore((s) => s.history);
  const materializedViews = usePlatformStore((s) => s.materializedViews);
  const ingestJob = usePlatformStore((s) => s.ingestJob);
  const tasks = usePlatformStore((s) => s.tasks);
  const loading = usePlatformStore((s) => s.loading);
  const refreshAll = usePlatformStore((s) => s.refreshAll);

  const totalRows = useMemo(
    () => tables.reduce((acc, t) => acc + t.rows, 0),
    [tables],
  );
  const ratio = cacheHitRatio(metrics);

  if (loading && !metrics) {
    return <p className="text-sm text-muted-foreground">Loading overview…</p>;
  }

  return (
    <>
      {ingestJob && (ingestJob.state === "running" || ingestJob.state === "done") && (
        <div className="mb-4">
          <IngestJobProgress job={ingestJob} compact />
        </div>
      )}

      <div className="mb-4 flex justify-end">
        <Button type="button" variant="outline" size="sm" onClick={() => void refreshAll()}>
          <RefreshCw className="size-4" />
          Refresh
        </Button>
        <Button type="button" variant="secondary" size="sm" className="ml-2" asChild>
          <Link href="/ingest">Open ingest</Link>
        </Button>
      </div>

      <section className="mb-4 grid grid-cols-6 gap-3">
        <MetricCard label="Tables" value={String(metrics?.table_count ?? 0)} />
        <MetricCard label="Rows Total" value={String(totalRows)} />
        <MetricCard label="Indexing" value={String(metrics?.indexing_tables ?? 0)} />
        <MetricCard
          label="Avg Latency"
          value={`${Math.round(metrics?.avg_query_latency_ms ?? 0)} ms`}
        />
        <MetricCard label="Jobs" value={String(jobs.length)} />
        <MetricCard label="Materialized views" value={String(materializedViews.length)} />
      </section>

      <div className="grid grid-cols-[2fr_1fr] gap-4">
        <Card>
          <CardHeader>
            <CardTitle>Latency Trend</CardTitle>
            <CardDescription>Recent query executions (relative)</CardDescription>
          </CardHeader>
          <CardContent className="space-y-2">
            {history.length === 0 ? (
              <p className="text-sm text-muted-foreground">No queries executed yet.</p>
            ) : (
              history.slice(0, 8).map((h, idx) => {
                const width = Math.max(8, Math.min(100, Math.round(h.latency_ms / 4)));
                return (
                  <div key={idx} className="space-y-1">
                    <div className="flex items-center justify-between text-xs text-muted-foreground">
                      <span className="truncate pr-3">{h.kind}</span>
                      <span>{Math.round(h.latency_ms)}ms</span>
                    </div>
                    <div className="h-2 rounded bg-muted">
                      <div className="h-2 rounded bg-primary" style={{ width: `${width}%` }} />
                    </div>
                  </div>
                );
              })
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>System Panels</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <StatusRow icon={Timer} label="Cache Hit" value={ratio} />
            <StatusRow icon={Database} label="Tables" value={String(metrics?.table_count ?? 0)} />
            <StatusRow
              icon={ArrowUpRight}
              label="Queries Executed"
              value={String(metrics?.query_count ?? 0)}
            />
            <StatusRow
              icon={ArrowUpRight}
              label="Indexing Tasks"
              value={String(metrics?.indexing_tables ?? 0)}
            />
            {ingestJob?.state === "running" && (
              <StatusRow
                icon={ArrowUpRight}
                label="HF ingest"
                value={ingestJob.phase ?? "running"}
              />
            )}
            {tasks.some((t) => t.state === "running") && (
              <StatusRow
                icon={ArrowUpRight}
                label="API tasks"
                value={String(tasks.filter((t) => t.state === "running").length)}
              />
            )}
          </CardContent>
        </Card>
      </div>

      {materializedViews.length > 0 && (
        <Card className="mt-4">
          <CardHeader className="flex-row items-center justify-between">
            <CardTitle className="text-base">Materialized views</CardTitle>
            <Button variant="outline" size="sm" asChild>
              <Link href="/views">View all</Link>
            </Button>
          </CardHeader>
          <CardContent className="space-y-1 text-sm">
            {materializedViews.slice(0, 5).map((mv) => (
              <div key={mv.name} className="flex justify-between rounded border border-border px-2 py-1">
                <span>{mv.name}</span>
                <span className="text-muted-foreground">{mv.row_count} rows</span>
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </>
  );
}

function MetricCard({ label, value }: { label: string; value: string }) {
  return (
    <Card>
      <CardHeader className="pb-1">
        <CardDescription>{label}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-semibold">{value}</div>
      </CardContent>
    </Card>
  );
}

function StatusRow({
  icon: Icon,
  label,
  value,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center justify-between rounded border border-border bg-muted/30 px-2 py-1.5 text-sm">
      <div className="flex items-center gap-2 text-muted-foreground">
        <Icon className="size-4" />
        <span>{label}</span>
      </div>
      <span className="font-medium">{value}</span>
    </div>
  );
}
