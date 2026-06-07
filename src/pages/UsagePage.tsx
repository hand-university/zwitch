import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Coins,
  DatabaseZap,
  HardDriveDownload,
  Loader2,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { message } from "@/components/ui/message";
import { clearUsage, getUsageSummary } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { DailyUsage, UsageSummary, UsageTotals } from "@/types";

interface UsagePageProps {
  busy: boolean;
  onError: (error: unknown) => void | Promise<void>;
}

const CALENDAR_WEEKS = 27;
const CALENDAR_CELL = 10;
const CALENDAR_GAP = 3;

/** GitHub 亮色主题色阶：无活动浅灰 → 用量越多绿色越深 */
const CALENDAR_COLORS = [
  "#ebedf0",
  "#9be9a8",
  "#40c463",
  "#30a14e",
  "#216e39",
] as const;


const WEEKDAY_LABELS: Record<number, string> = {
  1: "一",
  3: "三",
  5: "五",
};

function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return value.toLocaleString("en-US");
}

function formatFullNumber(value: number): string {
  return value.toLocaleString("en-US");
}

function formatCost(value: number): string {
  if (value === 0) return "$0.00";
  if (value < 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(2)}`;
}

function toDateKey(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

function parseLocalDate(dateStr: string): Date {
  const [y, m, d] = dateStr.split("-").map(Number);
  return new Date(y, m - 1, d);
}

function addDays(date: Date, days: number): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}

function localTodayKey(): string {
  return toDateKey(new Date());
}

export function UsagePage({ busy, onError }: UsagePageProps) {
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [clearing, setClearing] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setSummary(await getUsageSummary());
    } catch (e) {
      await onError(e);
    } finally {
      setLoading(false);
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleClear = async () => {
    setClearing(true);
    try {
      await clearUsage();
      await load();
      message.success("已清空用量统计");
    } catch (e) {
      await onError(e);
    } finally {
      setClearing(false);
    }
  };

  const total = summary?.total;

  if (loading && !summary) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center text-muted-foreground">
        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
        加载用量统计...
      </div>
    );
  }

  const hasData = total && total.requests > 0;

  return (
    <div className="p-6">
      <div className="mx-auto max-w-5xl space-y-5">
        <div className="flex items-center justify-end gap-2">
          <Button variant="outline" size="sm" onClick={() => void load()} disabled={busy || loading}>
            <RefreshCw className={loading ? "animate-spin" : ""} />
            刷新
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleClear}
            disabled={busy || clearing || !hasData}
          >
            {clearing ? <Loader2 className="animate-spin" /> : <Trash2 />}
            清空统计
          </Button>
        </div>

        {!hasData ? (
          <Card className="p-10 text-center text-sm text-muted-foreground">
            暂无用量数据。开启代理并通过 Codex / Claude Code 发起请求后，
            这里会记录 token 用量与费用。
          </Card>
        ) : (
          <>
            <SummaryCards summary={summary!} />
            <TokenBreakdown total={total!} />
            <ActiveCalendar summary={summary!} />
            <ModelTable summary={summary!} />
          </>
        )}
      </div>
    </div>
  );
}

function SummaryCards({ summary }: { summary: UsageSummary }) {
  const total = summary.total;
  const totalTokens =
    total.input_tokens +
    total.output_tokens +
    total.cache_creation_tokens +
    total.cache_read_tokens;

  const cards = [
    { label: "总消耗金额", value: formatCost(total.cost_usd), accent: "text-emerald-600 dark:text-emerald-400" },
    { label: "总 Token", value: formatTokens(totalTokens), sub: formatFullNumber(totalTokens) },
    { label: "请求次数", value: formatFullNumber(total.requests) },
    { label: "活跃天数", value: `${summary.active_days} 天` },
  ];

  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
      {cards.map((card) => (
        <Card key={card.label} className="p-4">
          <p className="text-xs text-muted-foreground">{card.label}</p>
          <p className={`mt-1 text-2xl font-semibold tracking-tight ${card.accent ?? ""}`}>
            {card.value}
          </p>
          {card.sub ? (
            <p className="mt-0.5 text-xs text-muted-foreground">{card.sub}</p>
          ) : null}
        </Card>
      ))}
    </div>
  );
}

function TokenBreakdown({ total }: { total: UsageTotals }) {
  const rows = [
    { label: "输入", value: total.input_tokens, icon: ArrowUpFromLine, color: "text-sky-500" },
    { label: "输出", value: total.output_tokens, icon: ArrowDownToLine, color: "text-violet-500" },
    {
      label: "创建缓存",
      value: total.cache_creation_tokens,
      icon: DatabaseZap,
      color: "text-amber-500",
    },
    {
      label: "读取缓存",
      value: total.cache_read_tokens,
      icon: HardDriveDownload,
      color: "text-emerald-500",
    },
  ];
  const sum = rows.reduce((acc, row) => acc + row.value, 0) || 1;

  return (
    <Card className="p-5">
      <div className="mb-4 flex items-center gap-2">
        <Coins className="h-4 w-4 text-muted-foreground" />
        <h3 className="text-sm font-medium">Token 用量构成</h3>
      </div>
      <div className="space-y-3">
        {rows.map((row) => {
          const Icon = row.icon;
          const pct = (row.value / sum) * 100;
          return (
            <div key={row.label}>
              <div className="mb-1 flex items-center justify-between text-sm">
                <span className="flex items-center gap-2">
                  <Icon className={`h-4 w-4 ${row.color}`} />
                  {row.label}
                </span>
                <span className="font-medium" title={formatFullNumber(row.value)}>
                  {formatTokens(row.value)}
                  <span className="ml-2 text-xs text-muted-foreground">
                    {pct.toFixed(1)}%
                  </span>
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-primary/70"
                  style={{ width: `${pct}%` }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </Card>
  );
}

type ContributionLevel = 0 | 1 | 2 | 3 | 4;

function dayTokens(day: DailyUsage): number {
  return (
    day.input_tokens +
    day.output_tokens +
    day.cache_creation_tokens +
    day.cache_read_tokens
  );
}

/** 按绝对 token 数分档（对数尺度），无活动浅灰，用量越多绿色越深 */
function contributionLevel(tokens: number): ContributionLevel {
  if (tokens <= 0) return 0;
  const log = Math.log10(tokens);
  if (log < 3) return 1;
  if (log < 4) return 2;
  if (log < 5) return 3;
  return 4;
}

function CalendarDayCell({
  level,
  isToday,
  title,
  future,
}: {
  level: ContributionLevel;
  isToday: boolean;
  title: string;
  future: boolean;
}) {
  if (future) {
    return (
      <div
        style={{ width: CALENDAR_CELL, height: CALENDAR_CELL }}
        className="shrink-0 rounded-[2px]"
      />
    );
  }

  return (
    <div
      title={title}
      style={{
        width: CALENDAR_CELL,
        height: CALENDAR_CELL,
        backgroundColor: CALENDAR_COLORS[level],
      }}
      className={cn(
        "shrink-0 rounded-[2px]",
        isToday &&
          "outline outline-1 outline-[rgba(27,31,36,0.15)] outline-offset-[-1px]",
      )}
    />
  );
}

function ActiveCalendar({ summary }: { summary: UsageSummary }) {
  const byDate = useMemo(() => {
    const map = new Map<string, DailyUsage>();
    for (const day of summary.calendar) map.set(day.date, day);
    return map;
  }, [summary.calendar]);

  const { weeks, monthLabels, todayKey } = useMemo(() => {
    const todayKey = summary.today || localTodayKey();
    const today = parseLocalDate(todayKey);
    const todayDow = today.getDay();
    const gridStart = addDays(today, -todayDow - (CALENDAR_WEEKS - 1) * 7);

    const weeks: { key: string; tokens: number; day: DailyUsage | undefined; future: boolean }[][] =
      [];
    const monthLabels: { col: number; label: string }[] = [];
    const labeledMonths = new Set<number>();

    for (let w = 0; w < CALENDAR_WEEKS; w += 1) {
      const week: typeof weeks[number] = [];
      for (let dow = 0; dow < 7; dow += 1) {
        const date = addDays(gridStart, w * 7 + dow);
        const key = toDateKey(date);
        const day = byDate.get(key);
        const tokens = day ? dayTokens(day) : 0;
        const future = date > today;
        week.push({ key, tokens, day, future });

        if (date.getDate() === 1) {
          const month = date.getMonth();
          if (!labeledMonths.has(month)) {
            labeledMonths.add(month);
            monthLabels.push({
              col: w,
              label: `${month + 1}月`,
            });
          }
        }
      }
      weeks.push(week);
    }
    return { weeks, monthLabels, todayKey };
  }, [byDate, summary.today]);

  return (
    <Card className="p-5">
      <div className="mb-4 flex items-center justify-between gap-4">
        <h3 className="text-sm font-medium">活跃日历</h3>
        <div className="flex items-center gap-1 text-xs text-muted-foreground">
          <span>少</span>
          {CALENDAR_COLORS.map((color) => (
            <span
              key={color}
              style={{
                width: CALENDAR_CELL,
                height: CALENDAR_CELL,
                backgroundColor: color,
              }}
              className="inline-block shrink-0 rounded-[2px]"
            />
          ))}
          <span>多</span>
        </div>
      </div>

      <div className="overflow-x-auto">
        <div className="inline-flex" style={{ gap: CALENDAR_GAP }}>
          <div
            className="flex shrink-0 flex-col text-[10px] leading-none text-muted-foreground"
            style={{ gap: CALENDAR_GAP, paddingTop: 15 }}
          >
            {Array.from({ length: 7 }, (_, dow) => (
              <div
                key={dow}
                className="flex items-center justify-end pr-1"
                style={{ height: CALENDAR_CELL }}
              >
                {WEEKDAY_LABELS[dow] ?? ""}
              </div>
            ))}
          </div>

          <div className="flex flex-col" style={{ gap: CALENDAR_GAP }}>
            <div className="flex" style={{ gap: CALENDAR_GAP, height: 15 }}>
              {weeks.map((_, col) => {
                const label = monthLabels.find((m) => m.col === col);
                return (
                  <div
                    key={col}
                    style={{ width: CALENDAR_CELL }}
                    className="relative overflow-visible"
                  >
                    {label ? (
                      <span className="absolute left-0 top-0 whitespace-nowrap text-[10px] text-muted-foreground">
                        {label.label}
                      </span>
                    ) : null}
                  </div>
                );
              })}
            </div>

            <div className="flex" style={{ gap: CALENDAR_GAP }}>
              {weeks.map((week, col) => (
                <div key={col} className="flex flex-col" style={{ gap: CALENDAR_GAP }}>
                  {week.map((cell) => {
                    const title = cell.day
                      ? `${cell.key}：${formatFullNumber(cell.tokens)} tokens · ${formatCost(cell.day.cost_usd)} · ${cell.day.requests} 次`
                      : `${cell.key}：无活动`;
                    return (
                      <CalendarDayCell
                        key={cell.key}
                        level={contributionLevel(cell.tokens)}
                        isToday={cell.key === todayKey}
                        title={title}
                        future={cell.future}
                      />
                    );
                  })}
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </Card>
  );
}

function ModelTable({ summary }: { summary: UsageSummary }) {
  if (summary.models.length === 0) return null;

  return (
    <Card className="overflow-hidden">
      <div className="border-b border-border px-5 py-3">
        <h3 className="text-sm font-medium">按模型统计</h3>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-xs text-muted-foreground">
              <th className="px-5 py-2.5 font-medium">模型</th>
              <th className="px-3 py-2.5 text-right font-medium">输入</th>
              <th className="px-3 py-2.5 text-right font-medium">输出</th>
              <th className="px-3 py-2.5 text-right font-medium">创建缓存</th>
              <th className="px-3 py-2.5 text-right font-medium">读取缓存</th>
              <th className="px-3 py-2.5 text-right font-medium">请求</th>
              <th className="px-5 py-2.5 text-right font-medium">费用</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {summary.models.map((model) => (
              <tr key={model.model} className="hover:bg-card/45">
                <td className="px-5 py-2.5 font-mono text-xs">{model.model}</td>
                <td className="px-3 py-2.5 text-right" title={formatFullNumber(model.input_tokens)}>
                  {formatTokens(model.input_tokens)}
                </td>
                <td className="px-3 py-2.5 text-right" title={formatFullNumber(model.output_tokens)}>
                  {formatTokens(model.output_tokens)}
                </td>
                <td
                  className="px-3 py-2.5 text-right"
                  title={formatFullNumber(model.cache_creation_tokens)}
                >
                  {formatTokens(model.cache_creation_tokens)}
                </td>
                <td
                  className="px-3 py-2.5 text-right"
                  title={formatFullNumber(model.cache_read_tokens)}
                >
                  {formatTokens(model.cache_read_tokens)}
                </td>
                <td className="px-3 py-2.5 text-right">{formatFullNumber(model.requests)}</td>
                <td className="px-5 py-2.5 text-right font-medium">{formatCost(model.cost_usd)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Card>
  );
}
