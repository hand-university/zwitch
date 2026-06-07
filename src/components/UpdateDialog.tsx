import { useCallback, useEffect, useState } from "react";
import { Download, Loader2, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { UpdateChangelog } from "@/components/UpdateChangelog";
import {
  deferDownloadedUpdate,
  downloadAvailableUpdate,
  installDownloadedUpdate,
  onUpdateDownloadProgress,
} from "@/lib/api";
import { message } from "@/components/ui/message";
import type { UpdateCheckResult, UpdateDownloadProgress } from "@/types";
import { cn } from "@/lib/utils";

type DialogPhase = "downloading" | "ready" | "installing" | "error";

interface UpdateDialogProps {
  open: boolean;
  updateInfo: UpdateCheckResult | null;
  onClose: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function UpdateDialog({ open, updateInfo, onClose }: UpdateDialogProps) {
  const [phase, setPhase] = useState<DialogPhase>("downloading");
  const [progress, setProgress] = useState<UpdateDownloadProgress>({
    downloaded: 0,
    total: null,
  });
  const [errorMessage, setErrorMessage] = useState("");

  const startDownload = useCallback(async () => {
    setPhase("downloading");
    setProgress({ downloaded: 0, total: null });
    setErrorMessage("");
    try {
      await downloadAvailableUpdate();
      setPhase("ready");
    } catch (error) {
      setPhase("error");
      setErrorMessage(String(error));
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    void startDownload();
  }, [open, startDownload]);

  useEffect(() => {
    if (!open || phase !== "downloading") return;
    let unlisten: (() => void) | undefined;
    void onUpdateDownloadProgress((payload) => {
      setProgress(payload);
    }).then((dispose) => {
      unlisten = dispose;
    });
    return () => {
      unlisten?.();
    };
  }, [open, phase]);

  const percent =
    progress.total && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;

  const handleRestart = async () => {
    setPhase("installing");
    try {
      await installDownloadedUpdate();
    } catch (error) {
      setPhase("ready");
      message.error(String(error));
    }
  };

  const handleDefer = async () => {
    try {
      await deferDownloadedUpdate();
      message.success("更新已就绪，将在下次启动时安装");
      onClose();
    } catch (error) {
      message.error(String(error));
    }
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <button
        type="button"
        aria-label="关闭"
        className="absolute inset-0 bg-black/25 backdrop-blur-[2px]"
        onClick={phase === "ready" || phase === "error" ? onClose : undefined}
        disabled={phase === "downloading" || phase === "installing"}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-dialog-title"
        className="glass-panel relative z-10 w-full max-w-md rounded-xl border p-6 shadow-lg"
      >
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 id="update-dialog-title" className="text-lg font-semibold">
              {phase === "ready" || phase === "installing"
                ? "更新已下载"
                : "正在下载更新"}
            </h2>
            {updateInfo?.version ? (
              <p className="mt-1 text-sm text-muted-foreground">
                新版本 v{updateInfo.version}
              </p>
            ) : null}
          </div>
          {phase === "ready" || phase === "error" ? (
            <Button variant="ghost" size="icon" onClick={onClose}>
              <X />
            </Button>
          ) : null}
        </div>

        {updateInfo?.notes ? (
          <div className="mt-4">
            <UpdateChangelog notes={updateInfo.notes} compact />
          </div>
        ) : null}

        <div className="mt-5 space-y-3">
          {phase === "downloading" ? (
            <>
              <div className="h-2 overflow-hidden rounded-full bg-secondary">
                <div
                  className={cn(
                    "h-full rounded-full bg-primary transition-[width] duration-200",
                    percent === null && "w-1/3 animate-pulse",
                  )}
                  style={percent !== null ? { width: `${percent}%` } : undefined}
                />
              </div>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <Loader2 className="size-3.5 animate-spin" />
                  下载中...
                </span>
                <span>
                  {formatBytes(progress.downloaded)}
                  {progress.total ? ` / ${formatBytes(progress.total)}` : ""}
                  {percent !== null ? ` (${percent}%)` : ""}
                </span>
              </div>
            </>
          ) : null}

          {phase === "ready" ? (
            <p className="text-sm text-muted-foreground">
              更新包已下载完成。你可以立即重启安装，或继续使用当前版本并在下次启动时安装。
            </p>
          ) : null}

          {phase === "installing" ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" />
              正在安装并重启...
            </div>
          ) : null}

          {phase === "error" ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </div>

        <div className="mt-6 flex flex-wrap justify-end gap-2">
          {phase === "ready" ? (
            <>
              <Button variant="outline" onClick={handleDefer}>
                下次启动时安装
              </Button>
              <Button onClick={handleRestart}>
                <RefreshCw />
                立即重启安装
              </Button>
            </>
          ) : null}

          {phase === "error" ? (
            <>
              <Button variant="outline" onClick={onClose}>
                取消
              </Button>
              <Button onClick={startDownload}>
                <Download />
                重试下载
              </Button>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
