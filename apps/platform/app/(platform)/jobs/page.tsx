"use client";

import { type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";

import { DataTable } from "@/components/data-table";
import { useToast } from "@/components/toast-provider";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { JobInfo, OpTask } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

export default function JobsPage() {
  const { toast } = useToast();
  const jobs = usePlatformStore((s) => s.jobs);
  const tasks = usePlatformStore((s) => s.tasks);
  const finishTableAction = usePlatformStore((s) => s.finishTableAction);
  const resumeTableAction = usePlatformStore((s) => s.resumeTableAction);

  const jobColumns = useMemo<ColumnDef<JobInfo>[]>(
    () => [
      { accessorKey: "table", header: "Table" },
      {
        accessorKey: "state",
        header: "State",
        cell: ({ row }) => (
          <Badge
            variant={
              row.original.state === "ready"
                ? "success"
                : row.original.state === "failed"
                  ? "warning"
                  : "secondary"
            }
          >
            {row.original.state}
          </Badge>
        ),
      },
      {
        accessorKey: "phase",
        header: "Phase",
        cell: ({ row }) => row.original.phase ?? "—",
      },
      {
        id: "progress",
        header: "Progress",
        cell: ({ row }) => {
          const j = row.original;
          if (j.segments_total <= 0) return "n/a";
          const pct = Math.round((j.segments_done / j.segments_total) * 100);
          return (
            <div className="min-w-[120px]">
              <span className="text-xs">
                {j.segments_done}/{j.segments_total}
              </span>
              <div className="mt-1 h-1.5 rounded bg-muted">
                <div
                  className="h-1.5 rounded bg-primary"
                  style={{ width: `${pct}%` }}
                />
              </div>
            </div>
          );
        },
      },
      {
        accessorKey: "message",
        header: "Message",
        cell: ({ row }) => (
          <span className="text-muted-foreground">{row.original.message ?? "—"}</span>
        ),
      },
      {
        id: "actions",
        header: "",
        cell: ({ row }) => (
          <div className="flex gap-1">
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => {
                void finishTableAction(row.original.table).then(() =>
                  toast({ title: "Finish task started" }),
                );
              }}
            >
              Finish
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => {
                void resumeTableAction(row.original.table).then(() =>
                  toast({ title: "Resume task started" }),
                );
              }}
            >
              Resume
            </Button>
          </div>
        ),
      },
    ],
    [finishTableAction, resumeTableAction, toast],
  );

  const taskColumns = useMemo<ColumnDef<OpTask>[]>(
    () => [
      { accessorKey: "id", header: "ID" },
      { accessorKey: "table", header: "Table" },
      { accessorKey: "kind", header: "Kind" },
      {
        accessorKey: "state",
        header: "State",
        cell: ({ row }) => (
          <Badge
            variant={
              row.original.state === "done"
                ? "success"
                : row.original.state === "failed"
                  ? "warning"
                  : "secondary"
            }
          >
            {row.original.state}
          </Badge>
        ),
      },
      {
        accessorKey: "message",
        header: "Message",
        cell: ({ row }) => row.original.message ?? "—",
      },
    ],
    [],
  );

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Index build status</CardTitle>
        </CardHeader>
        <CardContent>
          <DataTable columns={jobColumns} data={jobs} emptyMessage="No background jobs" />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>API tasks</CardTitle>
        </CardHeader>
        <CardContent>
          <DataTable columns={taskColumns} data={tasks} emptyMessage="No recent tasks" />
        </CardContent>
      </Card>
    </div>
  );
}
