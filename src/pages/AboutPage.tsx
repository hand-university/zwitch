import { useEffect, useState } from "react";
import { Download, Loader2, RefreshCw } from "lucide-react";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { UpdateDialog } from "@/components/UpdateDialog";
import {
  checkForUpdate,
  getAppInfo,
  getDownloadedUpdateInfo,
  installDownloadedUpdate,
} from "@/lib/api";
import { isDev } from "@/config/env";
import { message } from "@/components/ui/message";
import type { AppInfo, UpdateCheckResult } from "@/types";

type UpdateStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "ready"
  | "deferred"
  | "installing";

interface AboutPageProps {
  busy: boolean;
}

export function AboutPage({ busy }: AboutPageProps) {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResult | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  useEffect(() => {
    getAppInfo()
      .then(setAppInfo)
      .catch((e) => message.error(String(e)));
  }, []);

  useEffect(() => {
    if (isDev) return;
    getDownloadedUpdateInfo()
      .then((info) => {
        if (!info.ready || !info.version) return;
        if (info.deferred) {
          setUpdateStatus("deferred");
          setUpdateInfo((current) =>
            current ?? {
              available: true,
              currentVersion: appInfo?.version ?? "",
              version: info.version,
              notes: null,
              date: null,
            },
          );
        } else {
          setUpdateStatus("ready");
        }
      })
      .catch(() => {});
  }, [appInfo?.version]);

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

  const handleDownloadUpdate = () => {
    setDialogOpen(true);
  };

  const handleRestartInstall = async () => {
    setUpdateStatus("installing");
    try {
      await installDownloadedUpdate();
    } catch (e) {
      setUpdateStatus("ready");
      message.error(String(e));
    }
  };

  const checking = updateStatus === "checking";
  const installing = updateStatus === "installing";
  const canDownload = updateStatus === "available";
  const canRestart =
    updateStatus === "ready" || updateStatus === "deferred";

  return (
    <>
      <div className="p-6">
        <div className="mx-auto max-w-3xl">
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

            <div className="px-6 py-3.5">
              <div className="flex items-center justify-between gap-4">
                <span className="text-sm text-muted-foreground">版本</span>
                <div className="flex flex-wrap items-center justify-end gap-2">
                  <span className="text-sm font-medium">
                    {appInfo ? `v${appInfo.version}` : "—"}
                  </span>
                  {!isDev ? (
                    <>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={handleCheckUpdate}
                        disabled={busy || checking || installing || dialogOpen}
                      >
                        {checking ? (
                          <Loader2 className="animate-spin" />
                        ) : (
                          <RefreshCw />
                        )}
                        检查更新
                      </Button>
                      {canDownload ? (
                        <Button
                          size="sm"
                          onClick={handleDownloadUpdate}
                          disabled={busy || installing || dialogOpen}
                        >
                          <Download />
                          下载更新
                        </Button>
                      ) : null}
                      {canRestart ? (
                        <Button
                          size="sm"
                          onClick={handleRestartInstall}
                          disabled={busy || installing}
                        >
                          {installing ? (
                            <Loader2 className="animate-spin" />
                          ) : (
                            <RefreshCw />
                          )}
                          {installing ? "正在安装..." : "立即重启安装"}
                        </Button>
                      ) : null}
                    </>
                  ) : null}
                </div>
              </div>

              {updateStatus === "available" && updateInfo?.version ? (
                <p className="mt-2 text-right text-xs text-muted-foreground">
                  新版本 v{updateInfo.version} 可用
                </p>
              ) : null}
              {updateStatus === "deferred" && updateInfo?.version ? (
                <p className="mt-2 text-right text-xs text-muted-foreground">
                  已下载 v{updateInfo.version}，将在下次启动时安装
                </p>
              ) : null}
            </div>
          </Card>
        </div>
      </div>

      <UpdateDialog
        open={dialogOpen}
        updateInfo={updateInfo}
        onClose={() => {
          setDialogOpen(false);
          void getDownloadedUpdateInfo()
            .then((info) => {
              if (info.ready && info.deferred) {
                setUpdateStatus("deferred");
              } else if (info.ready) {
                setUpdateStatus("ready");
              }
            })
            .catch(() => {});
        }}
      />
    </>
  );
}
