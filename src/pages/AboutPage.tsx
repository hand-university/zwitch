import { useEffect, useState } from "react";
import { Download, Info, Loader2, RefreshCw } from "lucide-react";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { checkForUpdate, getAppInfo, installAvailableUpdate } from "@/lib/api";
import { isDev } from "@/config/env";
import { message } from "@/components/ui/message";
import type { AppInfo, UpdateCheckResult } from "@/types";

type UpdateStatus = "idle" | "checking" | "up-to-date" | "available" | "installing";

interface AboutPageProps {
  busy: boolean;
}

export function AboutPage({ busy }: AboutPageProps) {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResult | null>(null);

  useEffect(() => {
    getAppInfo()
      .then(setAppInfo)
      .catch((e) => message.error(String(e)));
  }, []);

  const handleCheckUpdate = async () => {
    setUpdateStatus("checking");
    setUpdateInfo(null);
    try {
      const result = await checkForUpdate();
      setUpdateInfo(result);
      if (result.available) {
        setUpdateStatus("available");
        message.info(`发现新版本 v${result.version}`);
      } else {
        setUpdateStatus("up-to-date");
        message.success("当前已是最新版本");
      }
    } catch (e) {
      setUpdateStatus("idle");
      message.error(String(e));
    }
  };

  const handleInstallUpdate = async () => {
    setUpdateStatus("installing");
    try {
      await installAvailableUpdate();
    } catch (e) {
      setUpdateStatus("available");
      message.error(String(e));
    }
  };

  const checking = updateStatus === "checking";
  const installing = updateStatus === "installing";

  return (
    <div className="p-6">
      <div className="mx-auto max-w-3xl space-y-4">
        <Card className="overflow-hidden">
          <div className="flex items-center gap-4 border-b border-border px-6 py-5">
            <AppIcon size="md" className="shadow-sm" />
            <div className="min-w-0 flex-1">
              <h3 className="text-lg font-semibold tracking-tight">
                {appInfo?.name ?? "ZWitch"}
              </h3>
              <p className="text-sm text-muted-foreground">
                AI CLI 代理配置与技能/插件市场
              </p>
            </div>
          </div>

          <div className="divide-y divide-border px-6">
            <InfoRow label="版本" value={appInfo ? `v${appInfo.version}` : "—"} />
            <InfoRow label="标识符" value={appInfo?.identifier ?? "—"} mono />
            <InfoRow
              label="运行环境"
              value={isDev ? "开发模式" : "正式版"}
            />
          </div>
        </Card>

        {!isDev ? (
        <Card className="p-6">
          <div className="flex items-start gap-4">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground">
              <Info className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1 space-y-3">
              <h4 className="text-sm font-medium">检查更新</h4>

              {updateStatus === "up-to-date" ? (
                <p className="text-sm text-emerald-600 dark:text-emerald-400">
                  当前 v{updateInfo?.currentVersion ?? appInfo?.version} 已是最新版本
                </p>
              ) : null}

              {updateStatus === "available" && updateInfo?.version ? (
                <div className="rounded-lg border border-border/60 bg-muted/30 p-3 text-sm">
                  <p className="font-medium">新版本 v{updateInfo.version} 可用</p>
                  {updateInfo.notes ? (
                    <p className="mt-1 whitespace-pre-wrap text-muted-foreground">
                      {updateInfo.notes}
                    </p>
                  ) : null}
                </div>
              ) : null}

              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleCheckUpdate}
                  disabled={busy || checking || installing}
                >
                  {checking ? (
                    <Loader2 className="animate-spin" />
                  ) : (
                    <RefreshCw />
                  )}
                  检查更新
                </Button>

                {updateStatus === "available" ? (
                  <Button
                    size="sm"
                    onClick={handleInstallUpdate}
                    disabled={busy || installing}
                  >
                    {installing ? (
                      <Loader2 className="animate-spin" />
                    ) : (
                      <Download />
                    )}
                    {installing ? "正在安装..." : "下载并安装"}
                  </Button>
                ) : null}
              </div>
            </div>
          </div>
        </Card>
        ) : null}
      </div>
    </div>
  );
}

function InfoRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-4 py-3.5">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span
        className={`text-sm font-medium ${mono ? "font-mono text-xs" : ""}`}
      >
        {value}
      </span>
    </div>
  );
}
