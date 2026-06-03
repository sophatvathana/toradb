"use client";

import { Copy, RotateCcw } from "lucide-react";

import { MessageParts } from "@/components/chat/message-parts";
import { Button } from "@/components/ui/button";
import type { ChatMessage } from "@/lib/chat/types";

export function MessageBubble({
  message,
  onRegenerate,
  onRetry,
}: {
  message: ChatMessage;
  onRegenerate?: () => void;
  onRetry?: () => void;
}) {
  const isUser = message.role === "user";
  const text = message.parts
    .filter((p) => p.type === "text")
    .map((p) => (p.type === "text" ? p.content : ""))
    .join("\n");

  const copy = () => {
    const parts = message.parts
      .map((p) => {
        if (p.type === "text") return p.content;
        if (p.type === "sql_result") return p.sql;
        return "";
      })
      .filter(Boolean)
      .join("\n\n");
    void navigator.clipboard.writeText(parts || text);
  };

  return (
    <div
      className={`flex ${isUser ? "justify-end" : "justify-start"}`}
    >
      <div
        className={`max-w-[min(100%,42rem)] rounded-lg px-4 py-3 ${
          isUser
            ? "bg-primary text-primary-foreground"
            : "border border-border bg-card"
        }`}
      >
        {isUser ? (
          <p className="whitespace-pre-wrap text-sm">{text}</p>
        ) : (
          <>
            <div aria-live={message.status === "streaming" ? "polite" : "off"}>
              <MessageParts parts={message.parts} onRetry={onRetry} />
            </div>
            {message.status === "pending" && (
              <p className="mt-2 text-xs text-muted-foreground animate-pulse">
                Thinking…
              </p>
            )}
          </>
        )}
        {!isUser && (
          <div className="mt-2 flex gap-1">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 px-2"
              onClick={copy}
            >
              <Copy className="size-3" />
              Copy
            </Button>
            {onRegenerate && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-7 px-2"
                onClick={onRegenerate}
              >
                <RotateCcw className="size-3" />
                Regenerate
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
