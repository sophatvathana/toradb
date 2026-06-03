"use client";

import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { SqlResponse } from "@/lib/api";

export type BarChartPoint = { label: string; value: number };

export type BarChartData = {
  title: string;
  labelCol: string;
  valueCol: string;
  points: BarChartPoint[];
  max: number;
};

/** Build bar chart data from a 2+ column SQL aggregate result. */
export function sqlResultToBarChart(
  result: SqlResponse,
  title?: string,
): BarChartData | null {
  if (result.columns.length < 2 || result.rows.length === 0) return null;
  const labelCol = result.columns[0];
  const valueCol = result.columns[result.columns.length - 1];
  const points = result.rows.map((r) => ({
    label: String(r[labelCol] ?? "—"),
    value: Number(r[valueCol] ?? 0),
  }));
  if (points.some((p) => Number.isNaN(p.value))) return null;
  const max = Math.max(...points.map((p) => p.value), 1);
  return {
    title: title ?? `${valueCol} by ${labelCol}`,
    labelCol,
    valueCol,
    points,
    max,
  };
}

export function chartSpecFromPoints(
  title: string,
  points: BarChartPoint[],
): BarChartData {
  const max = Math.max(...points.map((p) => p.value), 1);
  return {
    title,
    labelCol: "label",
    valueCol: "value",
    points,
    max,
  };
}

export function SqlBarChart({
  chart,
  compact,
}: {
  chart: BarChartData;
  compact?: boolean;
}) {
  const inner = (
    <div className="space-y-1.5">
      {chart.points.map((p, i) => (
        <div key={i} className="flex items-center gap-2 text-xs">
          <span
            className={`shrink-0 truncate font-mono ${compact ? "w-24" : "w-32"}`}
            title={p.label}
          >
            {p.label}
          </span>
          <div className="h-4 flex-1 overflow-hidden rounded-sm bg-muted">
            <div
              className="h-full rounded-sm bg-primary/70"
              style={{ width: `${Math.max(2, (p.value / chart.max) * 100)}%` }}
            />
          </div>
          <span className="w-16 shrink-0 text-right font-mono text-muted-foreground">
            {p.value.toLocaleString()}
          </span>
        </div>
      ))}
    </div>
  );

  if (compact) {
    return (
      <div className="rounded-md border border-border bg-muted/20 p-3">
        <p className="mb-2 text-sm font-medium">{chart.title}</p>
        {inner}
      </div>
    );
  }

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-3 space-y-0">
        <CardTitle className="text-base">{chart.title}</CardTitle>
        <Badge variant="outline">{chart.points.length} groups</Badge>
      </CardHeader>
      <CardContent>{inner}</CardContent>
    </Card>
  );
}
