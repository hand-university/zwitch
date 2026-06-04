import { ExternalLink, Info, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { CliToolStatus } from "@/types";
import { open } from "@tauri-apps/plugin-shell";

interface QuickStartPageProps {
  tools: CliToolStatus[];
  busy: boolean;
  proxyEnabled: boolean;
  onRefresh: () => void;
  onToolSwitch: (toolId: string, enabled: boolean) => void;
  onProxyToggle: (enabled: boolean) => void;
}

export function QuickStartPage({
  tools,
  busy,
  proxyEnabled,
  onRefresh,
  onToolSwitch,
  onProxyToggle,
}: QuickStartPageProps) {
  return (
    <div className="p-6">
      <div className="mx-auto max-w-3xl space-y-5">
        <div className="flex items-center justify-between px-1 pt-1">
          <div>
            <h2 className="text-base font-semibold tracking-tight">CLI 工具</h2>
            <p className="text-xs text-muted-foreground">
              为每个已安装的工具单独开启或关闭代理
            </p>
          </div>
          <Button variant="outline" size="sm" onClick={onRefresh} disabled={busy}>
            <RefreshCw className={busy ? "animate-spin" : ""} />
            刷新
          </Button>
        </div>

        <Card className="border-primary/30 bg-primary/5">
          <CardHeader>
            <div className="flex items-center justify-between gap-4">
              <div className="space-y-1">
                <CardTitle className="text-base">启用代理拦截</CardTitle>
                <CardDescription>
                  总开关：开启后才会将各 CLI 指向本地拦截服务；关闭则还原全部配置
                </CardDescription>
              </div>
              <Switch
                checked={proxyEnabled}
                onCheckedChange={onProxyToggle}
                disabled={busy}
              />
            </div>
          </CardHeader>
        </Card>

        <div className="space-y-4">
          {tools.map((tool) => (
            <Card key={tool.id} className={!proxyEnabled ? "opacity-60" : ""}>
              <CardHeader>
                <div className="flex items-center justify-between gap-4">
                  <div className="space-y-1">
                    <CardTitle className="text-base">{tool.name}</CardTitle>
                    <CardDescription>
                      {tool.installed ? "已安装" : "未检测到本地安装"}
                    </CardDescription>
                  </div>
                  {tool.installed ? (
                    <Switch
                      checked={tool.switch_enabled}
                      onCheckedChange={(checked) => onToolSwitch(tool.id, checked)}
                      disabled={busy || !proxyEnabled}
                    />
                  ) : (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => open(tool.install_url)}
                    >
                      <ExternalLink />
                      安装指南
                    </Button>
                  )}
                </div>
              </CardHeader>
            </Card>
          ))}
        </div>

        <div className="flex items-start gap-2 rounded-lg border border-border bg-muted/50 px-4 py-3">
          <Info className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <Label className="text-xs leading-relaxed text-muted-foreground">
            提示：修改配置后请重启对应的 CLI 工具使更改生效
          </Label>
        </div>
      </div>
    </div>
  );
}
