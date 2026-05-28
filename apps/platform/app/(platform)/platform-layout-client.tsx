"use client";

import { useEffect, useRef } from "react";

import { PlatformShell } from "@/components/platform-shell";
import { ToastProvider, useToast } from "@/components/toast-provider";
import { usePlatformStore } from "@/stores/platform-store";

function TaskToastWatcher() {
  const { toast } = useToast();
  const tasks = usePlatformStore((s) => s.tasks);
  const previousTasks = usePlatformStore((s) => s.previousTasks);
  const seen = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const task of tasks) {
      const key = `${task.id}:${task.state}`;
      if (seen.current.has(key)) continue;
      const prev = previousTasks.find((t) => t.id === task.id);
      if (prev?.state === "running" && (task.state === "done" || task.state === "failed")) {
        toast({
          title: task.state === "done" ? "Task completed" : "Task failed",
          description: `${task.kind} on ${task.table}`,
          variant: task.state === "failed" ? "error" : "default",
        });
      }
      seen.current.add(key);
    }
  }, [tasks, previousTasks, toast]);

  return null;
}

export function PlatformLayoutClient({ children }: { children: React.ReactNode }) {
  const hydrate = usePlatformStore((s) => s.hydrate);
  const startPolling = usePlatformStore((s) => s.startPolling);
  const stopPolling = usePlatformStore((s) => s.stopPolling);
  const stopIngestJobWatch = usePlatformStore((s) => s.stopIngestJobWatch);

  useEffect(() => {
    void hydrate();
    startPolling(5000);
    return () => {
      stopPolling();
      stopIngestJobWatch();
    };
  }, [hydrate, startPolling, stopPolling, stopIngestJobWatch]);

  return (
    <ToastProvider>
      <TaskToastWatcher />
      <PlatformShell>{children}</PlatformShell>
    </ToastProvider>
  );
}
