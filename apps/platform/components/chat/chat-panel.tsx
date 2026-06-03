"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { ChatHeader } from "@/components/chat/chat-header";
import { ChatInput } from "@/components/chat/chat-input";
import { MessageBubble } from "@/components/chat/message-bubble";
import { SetupBanner } from "@/components/chat/setup-banner";
import { useToast } from "@/components/toast-provider";
import { fetchChatConfig, isLlmConfigured } from "@/lib/chat/llm-client";
import { runChatTurn } from "@/lib/chat/run-turn";
import type { TurnProgress } from "@/lib/chat/types";
import { useChatStore } from "@/stores/chat-store";
import { useLlmSettingsStore } from "@/stores/llm-settings-store";
import { usePlatformStore } from "@/stores/platform-store";

export function ChatPanel() {
  const { toast } = useToast();
  const tables = usePlatformStore((s) => s.tables);
  const defaultTable = tables[0]?.name ?? "";

  const llmSettings = useLlmSettingsStore();
  const proxyAvailable = useLlmSettingsStore((s) => s.proxyAvailable);
  const setProxyMeta = useLlmSettingsStore((s) => s.setProxyMeta);

  const threads = useChatStore((s) => s.threads);
  const activeThreadId = useChatStore((s) => s.activeThreadId);
  const running = useChatStore((s) => s.running);
  const ensureActiveThread = useChatStore((s) => s.ensureActiveThread);
  const appendMessage = useChatStore((s) => s.appendMessage);
  const updateMessage = useChatStore((s) => s.updateMessage);
  const appendMessageParts = useChatStore((s) => s.appendMessageParts);
  const removeLastAssistant = useChatStore((s) => s.removeLastAssistant);
  const setRunning = useChatStore((s) => s.setRunning);
  const getActiveThread = useChatStore((s) => s.getActiveThread);
  const updateThreadMeta = useChatStore((s) => s.updateThreadMeta);

  const [input, setInput] = useState("");
  const [statusText, setStatusText] = useState("");
  const [lastUserText, setLastUserText] = useState("");

  const thread = getActiveThread();
  const llmReady = isLlmConfigured(llmSettings, proxyAvailable);

  useEffect(() => {
    void fetchChatConfig().then((c) => {
      setProxyMeta(c.proxy_available, c.default_model ?? null);
      if (c.default_model && !llmSettings.model) {
        useLlmSettingsStore.getState().setModel(c.default_model);
      }
    });
  }, [setProxyMeta, llmSettings.model]);

  useEffect(() => {
    if (defaultTable) ensureActiveThread(defaultTable);
  }, [defaultTable, ensureActiveThread]);

  useEffect(() => {
    if (thread && !thread.table && defaultTable) {
      updateThreadMeta(thread.id, { table: defaultTable });
    }
  }, [thread, defaultTable, updateThreadMeta]);

  const onProgress = useCallback((p: TurnProgress) => {
    if (p.phase === "thinking") setStatusText("Thinking…");
    else if (p.phase === "tool") setStatusText(`Running ${p.name}…`);
    else if (p.phase === "streaming") setStatusText("Writing answer…");
    else if (p.phase === "done") setStatusText("");
    else if (p.phase === "error") setStatusText(p.message);
  }, []);

  const sendMessage = useCallback(
    async (text: string, regenerate = false) => {
      if (!text.trim() || !llmReady) return;
      const t = getActiveThread();
      if (!t) return;
      if (!t.table && defaultTable) {
        updateThreadMeta(t.id, { table: defaultTable });
      }
      const activeTable = t.table || defaultTable;
      if (!activeTable) {
        toast({
          title: "No tables",
          description: "Ingest data or create a table first.",
          variant: "error",
        });
        return;
      }

      setLastUserText(text.trim());

      if (!regenerate) {
        appendMessage(t.id, {
          id: crypto.randomUUID(),
          role: "user",
          parts: [{ type: "text", content: text.trim() }],
          status: "done",
          createdAt: Date.now(),
        });
      } else {
        removeLastAssistant(t.id);
      }

      const assistantId = crypto.randomUUID();
      appendMessage(t.id, {
        id: assistantId,
        role: "assistant",
        parts: [{ type: "text", content: "" }],
        status: "streaming",
        createdAt: Date.now(),
      });

      const ac = new AbortController();
      setRunning(true, ac);
      setInput("");

      try {
        const current = getActiveThread();
        const prior =
          current?.messages.filter((m) => m.id !== assistantId) ?? [];

        const result = await runChatTurn({
          threadId: t.id,
          userText: text.trim(),
          table: activeTable,
          strategy: t.strategy,
          topK: t.topK,
          explain: t.explain,
          llmSettings,
          priorMessages: prior,
          signal: ac.signal,
          onProgress,
          onAssistantPart: (parts) => {
            appendMessageParts(t.id, assistantId, parts);
          },
          onStreamText: (text) => {
            updateMessage(t.id, assistantId, {
              parts: [{ type: "text", content: text }],
              status: "streaming",
            });
          },
        });

        updateMessage(t.id, assistantId, {
          parts: result.assistantMessage.parts,
          status: "done",
        });
      } catch (err) {
        if (ac.signal.aborted) {
          updateMessage(t.id, assistantId, {
            parts: [{ type: "error", message: "Stopped." }],
            status: "error",
          });
        } else {
          const message = err instanceof Error ? err.message : String(err);
          updateMessage(t.id, assistantId, {
            parts: [{ type: "error", message }],
            status: "error",
          });
          toast({ title: "Chat failed", description: message, variant: "error" });
        }
      } finally {
        setRunning(false, null);
        setStatusText("");
      }
    },
    [
      llmReady,
      getActiveThread,
      defaultTable,
      updateThreadMeta,
      appendMessage,
      removeLastAssistant,
      setRunning,
      llmSettings,
      onProgress,
      appendMessageParts,
      updateMessage,
      toast,
    ],
  );

  const stop = () => {
    const ac = useChatStore.getState().abortController;
    ac?.abort();
    setRunning(false, null);
    setStatusText("");
  };

  if (tables.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
        <p className="text-muted-foreground">No tables in this database yet.</p>
        <Link href="/ingest" className="text-sm text-primary hover:underline">
          Go to Ingest
        </Link>
        <Link href="/catalog" className="text-sm text-primary hover:underline">
          View Catalog
        </Link>
      </div>
    );
  }

  if (!thread) {
    return (
      <div className="flex flex-1 items-center justify-center text-muted-foreground">
        Select or create a thread
      </div>
    );
  }

  const lastAssistantIdx = [...thread.messages]
    .map((m, i) => (m.role === "assistant" ? i : -1))
    .filter((i) => i >= 0)
    .pop();

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ChatHeader thread={thread} onStop={stop} running={running} />
      <div className="flex-1 overflow-y-auto px-4 py-4">
        {!llmReady && <SetupBanner />}
        <div className="mx-auto flex max-w-3xl flex-col gap-4">
          {thread.messages.length === 0 && llmReady && (
            <p className="text-center text-sm text-muted-foreground">
              Ask questions about <span className="font-mono">{thread.table}</span>
              — search documents, run SQL analytics, or request a report.
            </p>
          )}
          {thread.messages.map((m, idx) => (
            <MessageBubble
              key={m.id}
              message={m}
              onRegenerate={
                m.role === "assistant" && idx === lastAssistantIdx && !running
                  ? () => void sendMessage(lastUserText, true)
                  : undefined
              }
              onRetry={
                m.parts.some((p) => p.type === "error") && lastUserText
                  ? () => void sendMessage(lastUserText, false)
                  : undefined
              }
            />
          ))}
        </div>
      </div>
      <ChatInput
        value={input}
        onChange={setInput}
        onSend={() => void sendMessage(input, false)}
        disabled={!llmReady || !thread.table}
        running={running}
        statusText={statusText}
      />
    </div>
  );
}
