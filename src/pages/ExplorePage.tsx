import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Cloud, Download, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { ExploreItem } from "@/types";

type PlatformFilter = "all" | "claude" | "codex";
type TypeFilter = "all" | "skill" | "plugin";

interface ExplorePageProps {
  items: ExploreItem[];
  busy: boolean;
  onInstall: (item: ExploreItem) => Promise<void>;
  onSearch?: () => Promise<void>;
}

const PLATFORM_LABELS: Record<ExploreItem["platform"], string> = {
  claude: "Claude",
  codex: "Codex",
};

const TYPE_LABELS: Record<ExploreItem["item_type"], string> = {
  skill: "技能",
  plugin: "插件",
};

export function ExplorePage({ items, busy, onInstall, onSearch }: ExplorePageProps) {
  const [platformFilter, setPlatformFilter] = useState<PlatformFilter>("all");
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  const [query, setQuery] = useState("");
  const [installingId, setInstallingId] = useState<string | null>(null);
  const skipSearchRefresh = useRef(true);

  useEffect(() => {
    if (!onSearch) {
      return;
    }
    if (skipSearchRefresh.current) {
      skipSearchRefresh.current = false;
      return;
    }
    const timer = window.setTimeout(() => {
      void onSearch();
    }, 300);
    return () => window.clearTimeout(timer);
  }, [query, onSearch]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return items.filter((item) => {
      if (typeFilter !== "all" && item.item_type !== typeFilter) {
        return false;
      }
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
  }, [items, platformFilter, typeFilter, query]);

  const stats = useMemo(
    () => ({
      total: items.length,
      skills: items.filter((item) => item.item_type === "skill").length,
      plugins: items.filter((item) => item.item_type === "plugin").length,
      installed: items.filter((item) => item.installed).length,
    }),
    [items],
  );

  const handleInstall = async (item: ExploreItem) => {
    setInstallingId(item.id);
    try {
      await onInstall(item);
    } finally {
      setInstallingId(null);
    }
  };

  return (
    <div className="p-6">
      <div className="mx-auto max-w-4xl space-y-5">
        <div className="grid gap-3 sm:grid-cols-4">
          {[
            { label: "全部", value: stats.total },
            { label: "技能", value: stats.skills },
            { label: "插件", value: stats.plugins },
            { label: "已安装", value: stats.installed },
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
            {(["all", "skill", "plugin"] as const).map((value) => (
              <button
                key={value}
                type="button"
                onClick={() => setTypeFilter(value)}
                className={`rounded-md px-3 py-1 text-xs transition-colors ${
                  typeFilter === value
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {value === "all" ? "全部类型" : TYPE_LABELS[value]}
              </button>
            ))}
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
                <CardTitle className="text-base">暂无可探索条目</CardTitle>
                <CardDescription>
                  {query.trim()
                    ? "没有匹配的条目，请尝试其他关键词。"
                    : "云端暂未分配可用的技能或插件。"}
                </CardDescription>
              </CardHeader>
            </Card>
          ) : (
            filtered.map((item) => (
              <Card key={item.id}>
                <CardHeader>
                  <div className="flex items-start justify-between gap-4">
                    <div className="min-w-0 space-y-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <CardTitle className="text-base">{item.name}</CardTitle>
                        <span className="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                          {PLATFORM_LABELS[item.platform]}
                        </span>
                        <span className="inline-flex items-center gap-1 rounded-md bg-primary/10 px-2 py-0.5 text-xs text-primary">
                          <Cloud className="h-3 w-3" />
                          {TYPE_LABELS[item.item_type]}
                        </span>
                        {item.version ? (
                          <span className="text-xs text-muted-foreground">
                            v{item.version}
                          </span>
                        ) : null}
                      </div>
                      <CardDescription>
                        {item.description ?? "无描述"}
                      </CardDescription>
                    </div>
                    {item.installed ? (
                      <span className="inline-flex shrink-0 items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
                        <Check className="h-3.5 w-3.5" />
                        已安装
                      </span>
                    ) : (
                      <Button
                        type="button"
                        size="sm"
                        disabled={busy || installingId === item.id}
                        onClick={() => void handleInstall(item)}
                      >
                        <Download />
                        安装
                      </Button>
                    )}
                  </div>
                </CardHeader>
              </Card>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
