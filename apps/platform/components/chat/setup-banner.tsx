"use client";

import Link from "next/link";

import { Button } from "@/components/ui/button";

export function SetupBanner() {
  return (
    <div
      className="rounded-md border border-amber-500/40 bg-amber-500/10 px-4 py-3 text-sm"
      role="alert"
    >
      <p className="font-medium">Configure an LLM to use Chat</p>
      <p className="mt-1 text-muted-foreground">
        Add an API key or enable the server LLM proxy in Settings.
      </p>
      <Button asChild size="sm" className="mt-2" variant="outline">
        <Link href="/settings">Open Settings</Link>
      </Button>
    </div>
  );
}
