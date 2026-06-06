import { useCallback, useEffect, useState } from "react";
import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";
import { cn } from "@/lib/utils";

type MessageType = "error" | "success" | "info";

interface MessageItem {
  id: number;
  type: MessageType;
  content: string;
}

const DISMISS_MS = 4500;

let pushMessage: ((type: MessageType, content: string) => void) | null = null;

function show(type: MessageType, content: string) {
  const text = content.trim();
  if (!text) return;
  pushMessage?.(type, text);
}

export const message = {
  error: (content: string) => show("error", content),
  success: (content: string) => show("success", content),
  info: (content: string) => show("info", content),
};

const TYPE_STYLES: Record<MessageType, string> = {
  error: "border-destructive/30 bg-background text-destructive",
  success: "border-emerald-500/30 bg-background text-emerald-700",
  info: "border-border bg-background text-foreground",
};

const TYPE_ICONS: Record<MessageType, typeof AlertCircle> = {
  error: AlertCircle,
  success: CheckCircle2,
  info: Info,
};

export function MessageHost() {
  const [messages, setMessages] = useState<MessageItem[]>([]);

  const dismiss = useCallback((id: number) => {
    setMessages((prev) => prev.filter((item) => item.id !== id));
  }, []);

  useEffect(() => {
    pushMessage = (type, content) => {
      const id = Date.now() + Math.random();
      setMessages((prev) => [...prev, { id, type, content }]);
      window.setTimeout(() => dismiss(id), DISMISS_MS);
    };
    return () => {
      pushMessage = null;
    };
  }, [dismiss]);

  if (messages.length === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none fixed top-4 left-1/2 z-[100] flex w-full max-w-md -translate-x-1/2 flex-col gap-2 px-4">
      {messages.map((item) => {
        const Icon = TYPE_ICONS[item.type];
        return (
          <div
            key={item.id}
            role="alert"
            className={cn(
              "pointer-events-auto flex items-start gap-2 rounded-lg border px-4 py-3 text-sm shadow-lg backdrop-blur-md",
              TYPE_STYLES[item.type],
            )}
          >
            <Icon className="mt-0.5 h-4 w-4 shrink-0" />
            <p className="min-w-0 flex-1 leading-relaxed">{item.content}</p>
            <button
              type="button"
              className="shrink-0 rounded p-0.5 opacity-60 transition-opacity hover:opacity-100"
              onClick={() => dismiss(item.id)}
              aria-label="关闭"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
