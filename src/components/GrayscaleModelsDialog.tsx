import { useEffect } from "react";
import { BookOpen, Sparkles, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  CLAUDE_GRAYSCALE_MODELS,
  CLAUDE_GRAYSCALE_USAGE_STEPS,
} from "@/content/claude-grayscale-models";
import { cn } from "@/lib/utils";

interface GrayscaleModelsDialogProps {
  open: boolean;
  onClose: () => void;
}

export function GrayscaleModelsDialog({
  open,
  onClose,
}: GrayscaleModelsDialogProps) {
  useEffect(() => {
    if (!open) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex flex-col">
      <button
        type="button"
        aria-label="关闭"
        className="absolute inset-0 bg-black/40 backdrop-blur-[2px]"
        onClick={onClose}
      />

      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="grayscale-models-dialog-title"
        className="glass-panel relative z-10 m-3 flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border shadow-2xl sm:m-4"
      >
        <header className="flex shrink-0 items-start justify-between gap-4 border-b border-border/60 px-6 py-5">
          <div className="space-y-1">
            <div className="flex flex-wrap items-center gap-2">
              <BookOpen className="h-5 w-5 text-violet-600 dark:text-violet-400" />
              <h2
                id="grayscale-models-dialog-title"
                className="text-lg font-semibold tracking-tight"
              >
                Claude Code 灰度模型使用说明
              </h2>
              <span className="inline-flex items-center rounded-full bg-violet-500/10 px-2 py-0.5 text-xs font-medium text-violet-700 dark:text-violet-400">
                灰度
              </span>
            </div>
            <p className="text-sm leading-relaxed text-muted-foreground">
              当前开放 claude-fable-5 灰度模型，通过 Claude CLI 自定义模型配置注入。仅对白名单用户可见。
            </p>
          </div>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="关闭说明">
            <X />
          </Button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <div className="mx-auto max-w-3xl space-y-6">
            <section className="space-y-3">
              <h3 className="text-sm font-semibold">快速上手</h3>
              <ol className="space-y-2.5">
                {CLAUDE_GRAYSCALE_USAGE_STEPS.map((step, index) => (
                  <li
                    key={step}
                    className="flex gap-3 text-sm leading-relaxed text-muted-foreground"
                  >
                    <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-violet-500/10 text-xs font-semibold text-violet-700 dark:text-violet-400">
                      {index + 1}
                    </span>
                    <span className="pt-0.5">{step}</span>
                  </li>
                ))}
              </ol>
            </section>

            <section className="space-y-3">
              <h3 className="text-sm font-semibold">可用模型</h3>
              <div className="grid gap-4">
                {CLAUDE_GRAYSCALE_MODELS.map((model) => (
                  <article
                    key={model.id}
                    className="rounded-xl border border-border/70 bg-muted/20 p-5"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <Sparkles className="h-4 w-4 text-violet-600 dark:text-violet-400" />
                      <h4 className="text-base font-semibold">{model.name}</h4>
                      {model.badge ? (
                        <span className="inline-flex items-center rounded-full bg-emerald-500/10 px-2 py-0.5 text-xs font-medium text-emerald-700 dark:text-emerald-400">
                          {model.badge}
                        </span>
                      ) : null}
                      <span className="inline-flex items-center rounded-full bg-violet-500/10 px-2 py-0.5 text-xs font-medium text-violet-700 dark:text-violet-400">
                        灰度
                      </span>
                    </div>

                    <p className="mt-3 text-sm leading-relaxed text-muted-foreground">
                      {model.summary}
                    </p>

                    <div className="mt-4 grid gap-4 sm:grid-cols-2">
                      <div>
                        <p className="text-xs font-medium text-foreground">特点</p>
                        <ul className="mt-2 space-y-1.5">
                          {model.highlights.map((item) => (
                            <li
                              key={item}
                              className="text-sm leading-relaxed text-muted-foreground before:mr-2 before:text-violet-500 before:content-['•']"
                            >
                              {item}
                            </li>
                          ))}
                        </ul>
                      </div>
                      <div>
                        <p className="text-xs font-medium text-foreground">适用场景</p>
                        <ul className="mt-2 space-y-1.5">
                          {model.suitableFor.map((item) => (
                            <li
                              key={item}
                              className="text-sm leading-relaxed text-muted-foreground before:mr-2 before:text-violet-500 before:content-['•']"
                            >
                              {item}
                            </li>
                          ))}
                        </ul>
                      </div>
                    </div>

                    <div className="mt-4 rounded-lg border border-border/60 bg-background/60 px-3 py-2">
                      <p className="text-xs text-muted-foreground">Claude Code 模型 ID</p>
                      <code className="mt-1 block break-all text-xs font-mono">
                        {model.claudeModelId}
                      </code>
                    </div>
                  </article>
                ))}
              </div>
            </section>

            <section
              className={cn(
                "rounded-xl border border-amber-500/20 bg-amber-500/[0.06] px-4 py-3",
              )}
            >
              <p className="text-sm leading-relaxed text-muted-foreground">
                配置变更后请重启 Claude Code。若看不到灰度模型，请确认已登录 ZWitch、代理处于运行中，且账号已在灰度白名单内。
              </p>
            </section>
          </div>
        </div>

        <footer className="flex shrink-0 justify-end border-t border-border/60 px-6 py-4">
          <Button onClick={onClose}>我知道了</Button>
        </footer>
      </div>
    </div>
  );
}
