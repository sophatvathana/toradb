"use client";

import { LoaderCircle, Send } from "lucide-react";
import { useCallback, useRef } from "react";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

export function ChatInput({
  value,
  onChange,
  onSend,
  disabled,
  running,
  statusText,
}: {
  value: string;
  onChange: (v: string) => void;
  onSend: () => void;
  disabled?: boolean;
  running?: boolean;
  statusText?: string;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        if (!disabled && !running && value.trim()) onSend();
      }
    },
    [disabled, running, value, onSend],
  );

  return (
    <div className="shrink-0 border-t border-border p-4">
      {statusText && (
        <p className="mb-2 text-xs text-muted-foreground" aria-live="polite">
          {statusText}
        </p>
      )}
      <div className="flex gap-2">
        <Textarea
          ref={ref}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Ask about your data…"
          className="min-h-[44px] max-h-40 resize-y"
          disabled={disabled || running}
          aria-label="Chat message"
        />
        <Button
          type="button"
          size="icon"
          className="shrink-0 self-end"
          disabled={disabled || running || !value.trim()}
          onClick={onSend}
          aria-label="Send message"
        >
          {running ? (
            <LoaderCircle className="size-4 animate-spin" />
          ) : (
            <Send className="size-4" />
          )}
        </Button>
      </div>
      <p className="mt-1 text-[10px] text-muted-foreground">
        Enter to send · Shift+Enter for newline
      </p>
    </div>
  );
}
