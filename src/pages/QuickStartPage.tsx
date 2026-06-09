import { Check, Copy, ExternalLink, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { message } from "@/components/ui/message";
import { openExternalUrl } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { CliToolStatus } from "@/types";

interface QuickStartPageProps {
  tools: CliToolStatus[];
  busy: boolean;
  proxyEnabled: boolean;
  onProxyToggle: (enabled: boolean) => void;
  onToolConfigToggle: (toolId: string, enabled: boolean) => void;
}

function configStatusText(tool: CliToolStatus, proxyEnabled: boolean): string | null {
  if (!tool.supported) {
    return null;
  }
  if (proxyEnabled && tool.config_enabled) {
    return "配置已注入";
  }
  if (proxyEnabled && !tool.config_enabled) {
    return "配置已关闭";
  }
  if (tool.config_enabled) {
    return "开启代理后将自动配置";
  }
  return "已关闭自动配置";
}

export function QuickStartPage({
  tools,
  busy,
  proxyEnabled,
  onProxyToggle,
  onToolConfigToggle,
}: QuickStartPageProps) {
  const enabledToolCount = tools.filter(
    (tool) => tool.supported && tool.config_enabled,
  ).length;

  const handleOpenDoc = async (url: string) => {
    try {
      await openExternalUrl(url);
    } catch (error) {
      message.error(String(error));
    }
  };

  const handleCopyInstallShell = async (shell: string) => {
    try {
      await navigator.clipboard.writeText(shell);
      message.success("已复制安装命令");
    } catch {
      message.error("复制失败");
    }
  };

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
                  ? `正在为 ${enabledToolCount} 个 CLI 工具自动配置代理`
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
            {tools.map((tool) => {
              const statusText = configStatusText(tool, proxyEnabled);

              return (
                <div
                  key={tool.id}
                  className="flex items-start justify-between gap-4 px-6 py-3.5"
                >
                  <div className="min-w-0 flex-1 space-y-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-medium">{tool.name}</span>
                      {tool.installed ? (
                        <span className="inline-flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400">
                          <Check className="h-3.5 w-3.5" />
                          已安装
                        </span>
                      ) : (
                        <span className="text-xs text-amber-600 dark:text-amber-400">
                          未安装
                        </span>
                      )}
                    </div>

                    {!tool.installed ? (
                      <div className="flex flex-wrap items-center gap-2">
                        <code
                          className="block max-w-full break-all rounded-md border border-border/60 bg-muted/40 px-2.5 py-1.5 text-xs font-mono leading-relaxed"
                        >
                          {tool.install_shell}
                        </code>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          disabled={busy}
                          onClick={() =>
                            handleCopyInstallShell(tool.install_shell)
                          }
                        >
                          <Copy />
                          复制
                        </Button>
                      </div>
                    ) : (
                      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                        {statusText ? (
                          <span className="text-xs text-muted-foreground">
                            {statusText}
                          </span>
                        ) : null}
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() =>
                            handleOpenDoc(tool.quick_start_doc_url)
                          }
                          className="inline-flex items-center gap-1 text-xs text-primary hover:underline disabled:opacity-50"
                        >
                          <ExternalLink className="h-3 w-3 shrink-0" />
                          快速开始文档
                        </button>
                      </div>
                    )}
                  </div>

                  {tool.supported ? (
                    <Switch
                      checked={tool.config_enabled}
                      onCheckedChange={(enabled) =>
                        onToolConfigToggle(tool.id, enabled)
                      }
                      disabled={busy}
                      className="shrink-0 data-[state=checked]:bg-emerald-500"
                    />
                  ) : (
                    <span className="shrink-0 text-xs text-muted-foreground">
                      暂不支持
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </Card>

        <p className="px-1 text-center text-xs text-muted-foreground">
          配置变更后请重启对应工具使更改生效
        </p>
      </div>
    </div>
  );
}
