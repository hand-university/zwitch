import { useCallback, useEffect, useState } from "react";
import { Download, Loader2, RefreshCw } from "lucide-react";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { UpdateChangelog } from "@/components/UpdateChangelog";
import { UpdateDialog } from "@/components/UpdateDialog";
import {
  checkForUpdate,
  getAppInfo,
  getDownloadedUpdateInfo,
  installDownloadedUpdate,
} from "@/lib/api";
import { isDev } from "@/config/env";
import { message } from "@/components/ui/message";
import type { AppInfo, DownloadedUpdateInfo, UpdateCheckResult } from "@/types";
import { cn } from "@/lib/utils";

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

function mergeUpdateState(
  check: UpdateCheckResult,
  downloaded: DownloadedUpdateInfo,
): { info: UpdateCheckResult; status: UpdateStatus } {
  if (downloaded.ready && downloaded.version) {
    return {
      info: {
        available: true,
        currentVersion: check.currentVersion,
        version: downloaded.version,
        notes: check.notes ?? downloaded.notes,
        date: check.date,
      },
      status: downloaded.deferred ? "deferred" : "ready",
    };
  }

  if (check.available) {
    return { info: check, status: "available" };
  }

  return { info: check, status: "up-to-date" };
}

export function AboutPage({ busy }: AboutPageProps) {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResult | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const refreshUpdateStatus = useCallback(async (silent = false) => {
    if (isDev) return;
    setUpdateStatus("checking");
    try {
      const [downloaded, check] = await Promise.all([
        getDownloadedUpdateInfo(),
        checkForUpdate(),
      ]);
      const { info, status } = mergeUpdateState(check, downloaded);
      setUpdateInfo(info);
      setUpdateStatus(status);
      if (!silent) {
        if (status === "available") {
          message.info(`发现新版本 v${info.version}`);
        } else if (status === "up-to-date") {
          message.success("当前已是最新版本");
        }
      }
    } catch (error) {
      setUpdateStatus("idle");
      if (!silent) {
        message.error(String(error));
      }
    }
  }, []);

  useEffect(() => {
    getAppInfo()
      .then(setAppInfo)
      .catch((e) => message.error(String(e)));
  }, []);

  useEffect(() => {
    void refreshUpdateStatus(true);
  }, [refreshUpdateStatus]);

  const handleCheckUpdate = () => {
    void refreshUpdateStatus(false);
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
  const hasNewVersion =
    updateStatus === "available" ||
    updateStatus === "ready" ||
    updateStatus === "deferred";

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
                  <span className="inline-flex items-center gap-2 text-sm font-medium">
                    <span className="relative inline-flex items-center">
                      {appInfo ? `v${appInfo.version}` : "—"}
                      {hasNewVersion ? (
                        <span
                          aria-label="有新版本"
                          className="absolute -right-1.5 -top-1 h-2 w-2 rounded-full bg-primary ring-2 ring-background"
                        />
                      ) : null}
                    </span>
                    {hasNewVersion && updateInfo?.version ? (
                      <span
                        className={cn(
                          "inline-flex items-center rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary",
                          checking && "opacity-60",
                        )}
                      >
                        新版本 v{updateInfo.version}
                      </span>
                    ) : null}
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

              {updateStatus === "deferred" && updateInfo?.version ? (
                <p className="mt-2 text-right text-xs text-muted-foreground">
                  已下载 v{updateInfo.version}，将在下次启动时安装
                </p>
              ) : null}

              {hasNewVersion && updateInfo?.notes ? (
                <div className="mt-4">
                  <UpdateChangelog notes={updateInfo.notes} />
                </div>
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
          void refreshUpdateStatus(true);
        }}
      />
    </>
  );
}
