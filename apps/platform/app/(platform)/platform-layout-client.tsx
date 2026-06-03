"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";

import { PlatformShell } from "@/components/platform-shell";
import { ToastProvider, useToast } from "@/components/toast-provider";
import { fetchAuthStatus, fetchConnections } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

/// Redirect to /login when auth is enabled and the current session is invalid.
function useAuthGuard(): boolean {
  const router = useRouter();
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const status = await fetchAuthStatus();
        if (!status.auth_enabled) {
          if (!cancelled) setReady(true);
          return;
        }
        // Auth enabled: probe a protected endpoint; 401 -> redirect to login.
        try {
          await fetchConnections();
          if (!cancelled) setReady(true);
        } catch {
          router.replace("/login");
        }
      } catch {
        if (!cancelled) setReady(true); // server unreachable; let pages surface errors
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [router]);
  return ready;
}

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

  const authReady = useAuthGuard();

  useEffect(() => {
    if (!authReady) return;
    void hydrate();
    startPolling(5000);
    return () => {
      stopPolling();
      stopIngestJobWatch();
    };
  }, [authReady, hydrate, startPolling, stopPolling, stopIngestJobWatch]);

  if (!authReady) {
    return (
      <div className="flex min-h-screen items-center justify-center text-sm text-muted-foreground">
        Loading…
      </div>
    );
  }

  return (
    <ToastProvider>
      <TaskToastWatcher />
      <PlatformShell>{children}</PlatformShell>
    </ToastProvider>
  );
}
