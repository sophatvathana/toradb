"use client";

import { LoaderCircle } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import type { IngestJob } from "@/lib/api";

type IngestJobProgressProps = {
  job: IngestJob;
  onCancel?: () => void;
  compact?: boolean;
};

function phaseLabel(job: IngestJob): string {
  if (job.phase) return job.phase;
  if (job.state === "done") return "Complete";
  if (job.state === "failed") return "Failed";
  return job.state;
}

export function IngestJobProgress({ job, onCancel, compact }: IngestJobProgressProps) {
  const running = job.state === "running";
  const done = job.state === "done";
  const failed = job.state === "failed" || job.state === "cancelled";

  return (
    <div
      className={`space-y-2 rounded-lg border border-border bg-muted/30 p-3 ${compact ? "text-xs" : "text-sm"}`}
      role="status"
      aria-live="polite"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          {running && <LoaderCircle className="size-4 animate-spin text-primary" />}
          <span className="font-medium">
            Ingest job #{job.id}
            <span className="ml-2 font-normal text-muted-foreground">
              ({job.source === "hf" ? "Hugging Face" : "file upload"})
            </span>
          </span>
        </div>
        <Badge
          variant={done ? "success" : failed ? "warning" : "secondary"}
        >
          {job.state}
        </Badge>
      </div>

      <p className="text-muted-foreground">
        Table <span className="font-mono text-foreground">{job.table}</span>
        {" · "}
        {phaseLabel(job)}
        {job.rows_ingested > 0 && (
          <span> · {job.rows_ingested.toLocaleString()} rows</span>
        )}
      </p>

      {job.message && failed && (
        <p className="text-destructive">{job.message}</p>
      )}

      {running && (
        <div className="space-y-1">
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>Progress</span>
            {job.progress != null ? (
              <span>{job.progress}%</span>
            ) : (
              <span className="animate-pulse">Working…</span>
            )}
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-muted">
            {job.progress != null ? (
              <div
                className="h-full rounded-full bg-primary transition-[width] duration-500 ease-out"
                style={{ width: `${Math.min(100, job.progress)}%` }}
              />
            ) : (
              <div className="h-full w-1/3 animate-pulse rounded-full bg-primary/70" />
            )}
          </div>
        </div>
      )}

      {!running && job.progress != null && (
        <div className="space-y-1">
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>Progress</span>
            <span>{job.progress}%</span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-primary"
              style={{ width: `${Math.min(100, job.progress)}%` }}
            />
          </div>
        </div>
      )}

      {running && onCancel && (
        <Button type="button" size="sm" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
      )}
    </div>
  );
}
