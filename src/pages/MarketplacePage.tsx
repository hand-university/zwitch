import { useMemo, useState } from "react";
import { Cloud, HardDrive, Search, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import type { MarketplaceItem } from "@/types";

type PlatformFilter = "all" | "claude" | "codex";

interface MarketplacePageProps {
  itemType: MarketplaceItem["item_type"];
  items: MarketplaceItem[];
  busy: boolean;
  onToggle: (item: MarketplaceItem, enabled: boolean) => void;
  onDelete: (item: MarketplaceItem) => Promise<void>;
}

interface DeleteConfirmDialogProps {
  item: MarketplaceItem;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

function DeleteConfirmDialog({
  item,
  busy,
  onConfirm,
  onCancel,
}: DeleteConfirmDialogProps) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
      <div className="w-full max-w-sm rounded-xl border border-border bg-white text-foreground shadow-lg">
        <CardHeader>
          <CardTitle className="text-base">确认删除</CardTitle>
          <CardDescription>
            确定删除「{item.name}」？将同时移除本地缓存与 CLI 安装目录中的文件。
          </CardDescription>
        </CardHeader>
        <div className="flex justify-end gap-2 px-6 pb-6">
          <Button
            type="button"
            variant="outline"
            onClick={onCancel}
            disabled={busy}
          >
            取消
          </Button>
          <Button
            type="button"
            variant="destructive"
            onClick={onConfirm}
            disabled={busy}
          >
            删除
          </Button>
        </div>
      </div>
    </div>
  );
}

const PLATFORM_LABELS: Record<MarketplaceItem["platform"], string> = {
  claude: "Claude",
  codex: "Codex",
};

const TYPE_LABELS: Record<MarketplaceItem["item_type"], string> = {
  skill: "技能",
  plugin: "插件",
};

const ORIGIN_LABELS: Record<MarketplaceItem["origin"], string> = {
  remote: "云端",
  local: "本地",
  both: "本地 + 云端",
};

function OriginBadge({ item }: { item: MarketplaceItem }) {
  if (item.origin === "local") {
    return (
      <span className="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
        <HardDrive className="h-3 w-3" />
        {ORIGIN_LABELS.local}
      </span>
    );
  }

  return (
    <span className="inline-flex items-center gap-1 rounded-md bg-primary/10 px-2 py-0.5 text-xs text-primary">
      <Cloud className="h-3 w-3" />
      {ORIGIN_LABELS[item.origin]}
    </span>
  );
}

export function MarketplacePage({
  itemType,
  items,
  busy,
  onToggle,
  onDelete,
}: MarketplacePageProps) {
  const [platformFilter, setPlatformFilter] = useState<PlatformFilter>("all");
  const [query, setQuery] = useState("");
  const [pendingDelete, setPendingDelete] = useState<MarketplaceItem | null>(
    null,
  );
  const typeLabel = TYPE_LABELS[itemType];
  const typeItems = useMemo(
    () => items.filter((item) => item.item_type === itemType),
    [items, itemType],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return typeItems.filter((item) => {
      if (platformFilter !== "all" && item.platform !== platformFilter) {
        return false;
      }
      if (!q) {
        return true;
      }
      return (
        item.name.toLowerCase().includes(q) ||
        (item.description?.toLowerCase().includes(q) ?? false)
      );
    });
  }, [typeItems, platformFilter, query]);

  const stats = useMemo(
    () => ({
      total: typeItems.length,
      enabled: typeItems.filter((item) => item.enabled).length,
      localOnly: typeItems.filter((item) => item.origin === "local").length,
      cloud: typeItems.filter((item) => item.remote_available).length,
    }),
    [typeItems],
  );

  return (
    <div className="p-6">
      <div className="mx-auto max-w-4xl space-y-5">
        <div className="grid gap-3 sm:grid-cols-4">
          {[
            { label: "全部", value: stats.total },
            { label: "已启用", value: stats.enabled },
            { label: "仅本地", value: stats.localOnly },
            { label: "云端", value: stats.cloud },
          ].map((stat) => (
            <Card key={stat.label}>
              <CardHeader className="py-3">
                <CardDescription>{stat.label}</CardDescription>
                <CardTitle className="text-2xl">{stat.value}</CardTitle>
              </CardHeader>
            </Card>
          ))}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-[200px] flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索名称或描述..."
              className="h-9 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm outline-none ring-offset-background focus-visible:ring-2 focus-visible:ring-ring"
            />
          </div>
          <div className="flex gap-1 rounded-lg border border-border p-1">
            {(["all", "claude", "codex"] as const).map((value) => (
              <button
                key={value}
                type="button"
                onClick={() => setPlatformFilter(value)}
                className={`rounded-md px-3 py-1 text-xs transition-colors ${
                  platformFilter === value
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {value === "all" ? "全部平台" : PLATFORM_LABELS[value]}
              </button>
            ))}
          </div>
        </div>

        <div className="space-y-3">
          {filtered.length === 0 ? (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">暂无条目</CardTitle>
                <CardDescription>
                  点击「一键同步市场」从云端拉取分配的 {typeLabel}，或点击「刷新」扫描本地已安装条目。
                </CardDescription>
              </CardHeader>
            </Card>
          ) : (
            filtered.map((item) => (
              <Card key={item.id} className={!item.enabled ? "opacity-70" : ""}>
                <CardHeader>
                  <div className="flex items-start justify-between gap-4">
                    <div className="min-w-0 space-y-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <CardTitle className="text-base">{item.name}</CardTitle>
                        <span className="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                          {PLATFORM_LABELS[item.platform]}
                        </span>
                        <OriginBadge item={item} />
                        {item.version ? (
                          <span className="text-xs text-muted-foreground">
                            v{item.version}
                          </span>
                        ) : null}
                      </div>
                      <CardDescription>
                        {item.description ?? "无描述"}
                      </CardDescription>
                      {!item.installed ? (
                        <p className="text-xs text-amber-600 dark:text-amber-400">
                          本地文件缺失，请重新同步
                        </p>
                      ) : null}
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="text-muted-foreground hover:text-destructive"
                        title="删除"
                        disabled={busy}
                        onClick={() => setPendingDelete(item)}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                      <Switch
                        checked={item.enabled}
                        onCheckedChange={(checked) => onToggle(item, checked)}
                        disabled={busy || !item.installed}
                      />
                    </div>
                  </div>
                </CardHeader>
              </Card>
            ))
          )}
        </div>

        <p className="px-1 text-center text-xs text-muted-foreground">
          配置变更后请重启对应工具使更改生效
        </p>
      </div>

      {pendingDelete ? (
        <DeleteConfirmDialog
          item={pendingDelete}
          busy={busy}
          onCancel={() => setPendingDelete(null)}
          onConfirm={() => {
            void onDelete(pendingDelete).finally(() => setPendingDelete(null));
          }}
        />
      ) : null}
    </div>
  );
}
