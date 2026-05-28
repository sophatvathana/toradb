"use client";

import * as Toast from "@radix-ui/react-toast";
import { createContext, useCallback, useContext, useState, type ReactNode } from "react";

type ToastItem = { id: number; title: string; description?: string; variant?: "default" | "error" };

type ToastContextValue = {
  toast: (item: Omit<ToastItem, "id">) => void;
};

const ToastContext = createContext<ToastContextValue | null>(null);

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const [open, setOpen] = useState(false);

  const toast = useCallback((item: Omit<ToastItem, "id">) => {
    const id = Date.now();
    setItems([{ ...item, id }]);
    setOpen(true);
  }, []);

  const current = items[0];

  return (
    <ToastContext.Provider value={{ toast }}>
      <Toast.Provider swipeDirection="right">
        {children}
        <Toast.Root
          open={open}
          onOpenChange={setOpen}
          className={`rounded-lg border px-4 py-3 shadow-lg ${
            current?.variant === "error"
              ? "border-destructive/50 bg-destructive/20"
              : "border-border bg-card"
          }`}
        >
          <Toast.Title className="text-sm font-semibold">{current?.title}</Toast.Title>
          {current?.description && (
            <Toast.Description className="mt-1 text-xs text-muted-foreground">
              {current.description}
            </Toast.Description>
          )}
        </Toast.Root>
        <Toast.Viewport className="fixed bottom-4 right-4 z-[100] flex max-w-sm flex-col gap-2" />
      </Toast.Provider>
    </ToastContext.Provider>
  );
}
