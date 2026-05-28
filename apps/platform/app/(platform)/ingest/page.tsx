"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { CloudDownload, Upload } from "lucide-react";

import { IngestJobProgress } from "@/components/ingest-job-progress";
import { useToast } from "@/components/toast-provider";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { HF_DATASET_PRESETS } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

export default function IngestPage() {
  const { toast } = useToast();
  const fileRef = useRef<HTMLInputElement>(null);
  const [step, setStep] = useState(1);
  const [fileQueue, setFileQueue] = useState<{ name: string; status: string }[]>([]);

  const ingestTable = usePlatformStore((s) => s.ingestTable);
  const ingestSource = usePlatformStore((s) => s.ingestSource);
  const ingestFormat = usePlatformStore((s) => s.ingestFormat);
  const ingestDropTable = usePlatformStore((s) => s.ingestDropTable);
  const ingestRowsIngested = usePlatformStore((s) => s.ingestRowsIngested);
  const ingestBulkActive = usePlatformStore((s) => s.ingestBulkActive);
  const ingestLimit = usePlatformStore((s) => s.ingestLimit);
  const hfDataset = usePlatformStore((s) => s.hfDataset);
  const hfConfig = usePlatformStore((s) => s.hfConfig);
  const hfSplit = usePlatformStore((s) => s.hfSplit);
  const hfTextColumn = usePlatformStore((s) => s.hfTextColumn);
  const error = usePlatformStore((s) => s.error);
  const ingestJob = usePlatformStore((s) => s.ingestJob);

  const setIngestTable = usePlatformStore((s) => s.setIngestTable);
  const setIngestSource = usePlatformStore((s) => s.setIngestSource);
  const setIngestFormat = usePlatformStore((s) => s.setIngestFormat);
  const setIngestDropTable = usePlatformStore((s) => s.setIngestDropTable);
  const setIngestLimit = usePlatformStore((s) => s.setIngestLimit);
  const setHfDataset = usePlatformStore((s) => s.setHfDataset);
  const setHfConfig = usePlatformStore((s) => s.setHfConfig);
  const setHfSplit = usePlatformStore((s) => s.setHfSplit);
  const setHfTextColumn = usePlatformStore((s) => s.setHfTextColumn);
  const beginIngestAction = usePlatformStore((s) => s.beginIngestAction);
  const uploadIngestFileAction = usePlatformStore((s) => s.uploadIngestFileAction);
  const ingestFromHfAction = usePlatformStore((s) => s.ingestFromHfAction);
  const watchIngestJob = usePlatformStore((s) => s.watchIngestJob);
  const cancelIngestJobAction = usePlatformStore((s) => s.cancelIngestJobAction);
  const finishIngestAction = usePlatformStore((s) => s.finishIngestAction);

  const jobRunning = ingestJob?.state === "running";

  useEffect(() => {
    if (ingestJob?.state === "done") {
      setStep(3);
    }
  }, [ingestJob?.state]);

  async function onBegin() {
    try {
      await beginIngestAction();
      setStep(2);
      toast({ title: "Ingest session started", description: ingestTable });
    } catch (err) {
      toast({
        title: "Begin failed",
        description: err instanceof Error ? err.message : String(err),
        variant: "error",
      });
    }
  }

  async function runUploadQueue(list: File[]) {
    for (let i = 0; i < list.length; i++) {
      const file = list[i];
      setFileQueue((q) =>
        q.map((item, idx) => (idx === i ? { ...item, status: "uploading" } : item)),
      );
      const jobId = await uploadIngestFileAction(file);
      await watchIngestJob(jobId);
      setFileQueue((q) =>
        q.map((item, idx) => (idx === i ? { ...item, status: "done" } : item)),
      );
    }
    setStep(3);
    toast({
      title: "All uploads complete",
      description: `${usePlatformStore.getState().ingestRowsIngested.toLocaleString()} rows total`,
    });
  }

  function onUpload() {
    const files = fileRef.current?.files;
    if (!files?.length) {
      toast({ title: "Choose a file", variant: "error" });
      return;
    }
    const list = Array.from(files);
    setFileQueue(list.map((f) => ({ name: f.name, status: "pending" })));
    setStep(2);
    toast({
      title: "Upload started",
      description: `${list.length} file(s) — track progress above`,
    });
    void runUploadQueue(list).catch((err) => {
      toast({
        title: "Upload failed",
        description: err instanceof Error ? err.message : String(err),
        variant: "error",
      });
    });
  }

  function onHfIngest() {
    setStep(2);
    void ingestFromHfAction()
      .then((jobId) => {
        toast({
          title: "HF ingest started",
          description: `Job #${jobId} — download & ingest in background`,
        });
        return watchIngestJob(jobId);
      })
      .then((job) => {
        toast({
          title: "Hugging Face ingest complete",
          description: `${job.rows_ingested.toLocaleString()} rows ingested`,
        });
      })
      .catch((err) => {
        toast({
          title: "HF ingest failed",
          description: err instanceof Error ? err.message : String(err),
          variant: "error",
        });
      });
  }

  async function onFinish() {
    try {
      await finishIngestAction(false);
      toast({ title: "Index build started", description: "See Jobs for progress" });
    } catch (err) {
      toast({
        title: "Finish failed",
        description: err instanceof Error ? err.message : String(err),
        variant: "error",
      });
    }
  }

  return (
    <div className="mx-auto max-w-2xl space-y-4">
      <div>
        <h2 className="text-xl font-semibold">Data ingest</h2>
        <p className="text-sm text-muted-foreground">
          Download and ingestion run in the background. Progress updates every second.
        </p>
      </div>

      {ingestJob && (
        <IngestJobProgress
          job={ingestJob}
          onCancel={
            jobRunning
              ? () => void cancelIngestJobAction(ingestJob.id)
              : undefined
          }
        />
      )}

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/20 p-2 text-sm">
          {error}
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Step 1 — Configure</CardTitle>
          <CardDescription>Table name and data source</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div>
            <label className="text-xs text-muted-foreground">Table name</label>
            <Input
              value={ingestTable}
              onChange={(e) => setIngestTable(e.target.value)}
              placeholder="passages"
              disabled={jobRunning}
            />
          </div>
          <div>
            <label className="text-xs text-muted-foreground">Source</label>
            <div className="mt-1 flex gap-2">
              <Button
                type="button"
                size="sm"
                variant={ingestSource === "file" ? "default" : "outline"}
                onClick={() => setIngestSource("file")}
                disabled={jobRunning}
              >
                <Upload className="size-4" />
                File upload
              </Button>
              <Button
                type="button"
                size="sm"
                variant={ingestSource === "hf" ? "default" : "outline"}
                onClick={() => setIngestSource("hf")}
                disabled={jobRunning}
              >
                <CloudDownload className="size-4" />
                Hugging Face
              </Button>
            </div>
          </div>
          {ingestSource === "file" && (
            <div>
              <label className="text-xs text-muted-foreground">File format</label>
              <select
                className="mt-1 flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                value={ingestFormat}
                onChange={(e) => setIngestFormat(e.target.value as "parquet" | "jsonl")}
                disabled={jobRunning}
              >
                <option value="jsonl">JSONL</option>
                <option value="parquet">Parquet</option>
              </select>
            </div>
          )}
          <div>
            <label className="text-xs text-muted-foreground">Row limit (0 = unlimited)</label>
            <Input
              type="number"
              min={0}
              value={ingestLimit}
              onChange={(e) => setIngestLimit(Number(e.target.value) || 0)}
              disabled={jobRunning}
            />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={ingestDropTable}
              onChange={(e) => setIngestDropTable(e.target.checked)}
              disabled={jobRunning}
            />
            Drop existing table directory first
          </label>
          <Button
            type="button"
            onClick={() => void onBegin()}
            disabled={!ingestTable || jobRunning}
          >
            Begin ingest session
          </Button>
        </CardContent>
      </Card>

      {ingestSource === "file" ? (
        <Card>
          <CardHeader>
            <CardTitle>Step 2 — Upload file</CardTitle>
            <CardDescription>
              {ingestBulkActive ? "Bulk session active" : "Begin session first"}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <input
              ref={fileRef}
              type="file"
              multiple
              accept={ingestFormat === "parquet" ? ".parquet" : ".jsonl,.json"}
              disabled={jobRunning}
            />
            {fileQueue.length > 0 && (
              <ul className="space-y-1 text-xs text-muted-foreground">
                {fileQueue.map((f) => (
                  <li key={f.name}>
                    {f.name}: {f.status}
                  </li>
                ))}
              </ul>
            )}
            <Button
              type="button"
              variant="secondary"
              disabled={jobRunning || step < 2}
              onClick={() => void onUpload()}
            >
              Upload &amp; ingest (background)
            </Button>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>Step 2 — Stream from Hugging Face</CardTitle>
            <CardDescription>
              Downloads Parquet/JSONL shards in parallel, then ingests in the background.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div>
              <label className="text-xs text-muted-foreground">Preset</label>
              <select
                className="mt-1 flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                defaultValue=""
                disabled={jobRunning}
                onChange={(e) => {
                  const preset = HF_DATASET_PRESETS.find((p) => p.label === e.target.value);
                  if (preset) {
                    setHfDataset(preset.dataset);
                    setHfConfig(preset.config);
                    setHfSplit(preset.split);
                    setHfTextColumn(preset.text_column);
                  }
                }}
              >
                <option value="">— custom —</option>
                {HF_DATASET_PRESETS.map((p) => (
                  <option key={p.label} value={p.label}>
                    {p.label}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="text-xs text-muted-foreground">Dataset id</label>
              <Input
                value={hfDataset}
                onChange={(e) => setHfDataset(e.target.value)}
                placeholder="Tevatron/msmarco-passage-corpus"
                disabled={jobRunning}
              />
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="text-xs text-muted-foreground">Config (optional)</label>
                <Input
                  value={hfConfig}
                  onChange={(e) => setHfConfig(e.target.value)}
                  placeholder="default"
                  disabled={jobRunning}
                />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">Split</label>
                <Input
                  value={hfSplit}
                  onChange={(e) => setHfSplit(e.target.value)}
                  placeholder="train"
                  disabled={jobRunning}
                />
              </div>
            </div>
            <div>
              <label className="text-xs text-muted-foreground">Text column</label>
              <Input
                value={hfTextColumn}
                onChange={(e) => setHfTextColumn(e.target.value)}
                placeholder="text"
                disabled={jobRunning}
              />
            </div>
            <Button
              type="button"
              variant="secondary"
              disabled={jobRunning || step < 2 || !hfDataset.trim()}
              onClick={() => void onHfIngest()}
            >
              Start background ingest
            </Button>
          </CardContent>
        </Card>
      )}

      {ingestRowsIngested > 0 && (
        <p className="text-sm text-muted-foreground">
          Last ingest: {ingestRowsIngested.toLocaleString()} rows
        </p>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Step 3 — Build indexes</CardTitle>
          <CardDescription>Runs in background; monitor on Jobs</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          <Button type="button" onClick={() => void onFinish()} disabled={step < 2 || jobRunning}>
            Finish &amp; build indexes
          </Button>
          <Button type="button" variant="outline" asChild>
            <Link href="/jobs">View jobs</Link>
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
