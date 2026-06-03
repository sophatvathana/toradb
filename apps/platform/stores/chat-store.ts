import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { ChatMessage, ChatPart, ChatThread } from "@/lib/chat/types";

function newThread(table = ""): ChatThread {
  const now = Date.now();
  return {
    id: crypto.randomUUID(),
    title: "New chat",
    table,
    strategy: "",
    topK: 10,
    explain: false,
    messages: [],
    createdAt: now,
    updatedAt: now,
  };
}

type ChatState = {
  threads: ChatThread[];
  activeThreadId: string | null;
  running: boolean;
  abortController: AbortController | null;
  ensureActiveThread: (defaultTable: string) => string;
  newThread: (table?: string) => void;
  deleteThread: (id: string) => void;
  renameThread: (id: string, title: string) => void;
  setActiveThread: (id: string) => void;
  updateThreadMeta: (
    id: string,
    patch: Partial<Pick<ChatThread, "table" | "strategy" | "topK" | "explain">>,
  ) => void;
  appendMessage: (threadId: string, message: ChatMessage) => void;
  updateMessage: (
    threadId: string,
    messageId: string,
    patch: Partial<Pick<ChatMessage, "parts" | "status">>,
  ) => void;
  appendMessageParts: (threadId: string, messageId: string, parts: ChatPart[]) => void;
  clearThread: (id: string) => void;
  removeLastAssistant: (threadId: string) => void;
  setRunning: (running: boolean, abortController?: AbortController | null) => void;
  getActiveThread: () => ChatThread | null;
};

export const useChatStore = create<ChatState>()(
  persist(
    (set, get) => ({
      threads: [],
      activeThreadId: null,
      running: false,
      abortController: null,

      ensureActiveThread: (defaultTable) => {
        const state = get();
        if (state.activeThreadId) {
          const t = state.threads.find((x) => x.id === state.activeThreadId);
          if (t) return t.id;
        }
        const t = newThread(defaultTable);
        set({
          threads: [t, ...state.threads],
          activeThreadId: t.id,
        });
        return t.id;
      },

      newThread: (table = "") => {
        const t = newThread(table);
        set((s) => ({
          threads: [t, ...s.threads],
          activeThreadId: t.id,
        }));
      },

      deleteThread: (id) => {
        set((s) => {
          const threads = s.threads.filter((t) => t.id !== id);
          const activeThreadId =
            s.activeThreadId === id ? (threads[0]?.id ?? null) : s.activeThreadId;
          return { threads, activeThreadId };
        });
      },

      renameThread: (id, title) => {
        set((s) => ({
          threads: s.threads.map((t) =>
            t.id === id ? { ...t, title, updatedAt: Date.now() } : t,
          ),
        }));
      },

      setActiveThread: (id) => set({ activeThreadId: id }),

      updateThreadMeta: (id, patch) => {
        set((s) => ({
          threads: s.threads.map((t) =>
            t.id === id ? { ...t, ...patch, updatedAt: Date.now() } : t,
          ),
        }));
      },

      appendMessage: (threadId, message) => {
        set((s) => ({
          threads: s.threads.map((t) => {
            if (t.id !== threadId) return t;
            const messages = [...t.messages, message];
            let title = t.title;
            if (
              t.title === "New chat" &&
              message.role === "user" &&
              message.parts[0]?.type === "text"
            ) {
              title = message.parts[0].content.slice(0, 48) || "New chat";
            }
            return { ...t, messages, title, updatedAt: Date.now() };
          }),
        }));
      },

      updateMessage: (threadId, messageId, patch) => {
        set((s) => ({
          threads: s.threads.map((t) =>
            t.id === threadId
              ? {
                  ...t,
                  messages: t.messages.map((m) =>
                    m.id === messageId ? { ...m, ...patch } : m,
                  ),
                  updatedAt: Date.now(),
                }
              : t,
          ),
        }));
      },

      appendMessageParts: (threadId, messageId, parts) => {
        set((s) => ({
          threads: s.threads.map((t) =>
            t.id === threadId
              ? {
                  ...t,
                  messages: t.messages.map((m) =>
                    m.id === messageId
                      ? { ...m, parts: [...m.parts, ...parts] }
                      : m,
                  ),
                  updatedAt: Date.now(),
                }
              : t,
          ),
        }));
      },

      clearThread: (id) => {
        set((s) => ({
          threads: s.threads.map((t) =>
            t.id === id ? { ...t, messages: [], updatedAt: Date.now() } : t,
          ),
        }));
      },

      removeLastAssistant: (threadId) => {
        set((s) => ({
          threads: s.threads.map((t) => {
            if (t.id !== threadId) return t;
            const messages = [...t.messages];
            for (let i = messages.length - 1; i >= 0; i--) {
              if (messages[i].role === "assistant") {
                messages.splice(i, 1);
                break;
              }
            }
            return { ...t, messages, updatedAt: Date.now() };
          }),
        }));
      },

      setRunning: (running, abortController = null) =>
        set({ running, abortController }),

      getActiveThread: () => {
        const { threads, activeThreadId } = get();
        return threads.find((t) => t.id === activeThreadId) ?? null;
      },
    }),
    {
      name: "toradb-chat",
      partialize: (s) => ({
        threads: s.threads,
        activeThreadId: s.activeThreadId,
      }),
    },
  ),
);
