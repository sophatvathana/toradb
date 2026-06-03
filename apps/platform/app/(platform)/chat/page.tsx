"use client";

import { ChatPanel } from "@/components/chat/chat-panel";
import { ThreadSidebar } from "@/components/chat/thread-sidebar";
import { usePlatformStore } from "@/stores/platform-store";

export default function ChatPage() {
  const tables = usePlatformStore((s) => s.tables);
  const defaultTable = tables[0]?.name ?? "";

  return (
    <div className="-m-4 flex h-[calc(100dvh-3.5rem)] min-h-[480px] flex-col md:-m-6">
      <div className="flex min-h-0 flex-1 overflow-hidden rounded-lg border border-border bg-background">
        <ThreadSidebar defaultTable={defaultTable} />
        <main className="flex min-w-0 flex-1 flex-col" aria-label="Chat conversation">
          <ChatPanel />
        </main>
      </div>
    </div>
  );
}
