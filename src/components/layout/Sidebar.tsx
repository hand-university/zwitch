import { Button } from "@/components/ui/button";
import { isDev } from "@/config/env";
import { cn } from "@/lib/utils";
import type { AuthState, NavItem } from "@/types";
import { LogOut, Rocket, Zap } from "lucide-react";

const NAV_ICONS = {
  "quick-start": Rocket,
} as const;

interface SidebarProps {
  auth: AuthState;
  items: NavItem[];
  activePage: NavItem["id"];
  onNavigate: (page: NavItem["id"]) => void;
  onLogout?: () => void;
}

function UserAvatar({ auth }: { auth: AuthState }) {
  const initials = (auth.user_name ?? "U").slice(0, 1).toUpperCase();

  if (auth.avatar) {
    return (
      <img
        src={auth.avatar}
        alt={auth.user_name ?? "用户头像"}
        className="h-9 w-9 rounded-full object-cover"
      />
    );
  }

  return (
    <div className="flex h-9 w-9 items-center justify-center rounded-full bg-primary text-sm font-medium text-primary-foreground">
      {initials}
    </div>
  );
}

export function Sidebar({
  auth,
  items,
  activePage,
  onNavigate,
  onLogout,
}: SidebarProps) {
  return (
    <aside className="flex h-full w-64 shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="flex items-center gap-3 px-5 py-5">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm">
          <Zap className="h-5 w-5" />
        </div>
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold tracking-tight">
            ZD Switch
          </p>
          <p className="truncate text-xs text-muted-foreground">AI CLI 代理配置</p>
        </div>
      </div>

      <nav className="flex-1 space-y-1 px-3 py-2">
        {items.map((item) => {
          const Icon = NAV_ICONS[item.id];
          const active = activePage === item.id;

          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onNavigate(item.id)}
              className={cn(
                "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-sm transition-all",
                active
                  ? "bg-card font-medium text-foreground shadow-sm ring-1 ring-border"
                  : "text-muted-foreground hover:bg-card/60 hover:text-foreground",
              )}
            >
              <Icon className="h-4 w-4 shrink-0" />
              {item.label}
            </button>
          );
        })}
      </nav>

      <div className="p-3">
        <div className="flex items-center gap-3 rounded-xl bg-card p-2.5 shadow-sm ring-1 ring-border">
          <UserAvatar auth={auth} />
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium">
              {auth.user_name ?? "未命名用户"}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              {auth.department ?? auth.title ?? "—"}
            </p>
          </div>
          {isDev && onLogout ? (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="shrink-0 text-muted-foreground hover:text-foreground"
              title="退出登录（开发）"
              onClick={onLogout}
            >
              <LogOut className="h-4 w-4" />
            </Button>
          ) : null}
        </div>
      </div>
    </aside>
  );
}
