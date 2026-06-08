import { Check, Zap } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import type { CliToolStatus } from "@/types";

interface QuickStartPageProps {
  tools: CliToolStatus[];
  busy: boolean;
  proxyEnabled: boolean;
  onProxyToggle: (enabled: boolean) => void;
}

export function QuickStartPage({
  tools,
  busy,
  proxyEnabled,
  onProxyToggle,
}: QuickStartPageProps) {
  const supportedCount = tools.filter((tool) => tool.supported).length;

  return (
    <div className="p-6">
      <div className="mx-auto max-w-3xl space-y-4">
        <div
          className={cn(
            "relative overflow-hidden rounded-2xl border p-5 transition-all duration-300",
            proxyEnabled
              ? "glass-surface border-emerald-500/25 bg-gradient-to-br from-emerald-500/[0.12] via-card/50 to-card/40 ring-1 ring-emerald-500/10"
              : "glass-surface border-border/50",
          )}
        >
          {proxyEnabled ? (
            <div className="pointer-events-none absolute -right-8 -top-8 h-32 w-32 rounded-full bg-emerald-500/10 blur-2xl" />
          ) : null}

          <div className="relative flex items-center gap-4">
            <div
              className={cn(
                "flex h-12 w-12 shrink-0 items-center justify-center rounded-xl transition-all duration-300",
                proxyEnabled
                  ? "bg-emerald-500 text-white shadow-md shadow-emerald-500/25"
                  : "bg-muted text-muted-foreground",
              )}
            >
              <Zap className="h-5 w-5" />
            </div>

            <div className="min-w-0 flex-1 space-y-1">
              <div className="flex flex-wrap items-center gap-2">
                <h3 className="text-base font-semibold tracking-tight">启用代理</h3>
                <span
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium",
                    proxyEnabled
                      ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"
                      : "bg-muted text-muted-foreground",
                  )}
                >
                  <span
                    className={cn(
                      "h-1.5 w-1.5 rounded-full",
                      proxyEnabled
                        ? "bg-emerald-500 animate-pulse"
                        : "bg-muted-foreground/50",
                    )}
                  />
                  {proxyEnabled ? "运行中" : "未开启"}
                </span>
              </div>
              <p className="text-sm leading-relaxed text-muted-foreground">
                {proxyEnabled
                  ? `正在为 ${supportedCount} 个支持的 CLI 工具自动配置代理`
                  : "开启后自动写入配置文件，关闭则恢复原始设置"}
              </p>
            </div>

            <Switch
              checked={proxyEnabled}
              onCheckedChange={onProxyToggle}
              disabled={busy}
              className="shrink-0 scale-110 data-[state=checked]:bg-emerald-500"
            />
          </div>
        </div>

        <Card>
          <div className="divide-y divide-border">
            {tools.map((tool) => (
              <div
                key={tool.id}
                className="flex items-center justify-between gap-4 px-6 py-3.5"
              >
                <span className="text-sm font-medium">{tool.name}</span>
                {tool.supported ? (
                  <span className="inline-flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
                    <Check className="h-3.5 w-3.5" />
                    支持
                  </span>
                ) : (
                  <span className="text-xs text-muted-foreground">暂不支持</span>
                )}
              </div>
            ))}
          </div>
        </Card>

        <p className="px-1 text-center text-xs text-muted-foreground">
          配置变更后请重启对应工具使更改生效
        </p>
      </div>
    </div>
  );
}
