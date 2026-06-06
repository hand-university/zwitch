import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { AuthState, NavItem } from "@/types";
import { Compass, Info, LogOut, Plug, Rocket, Sparkles } from "lucide-react";

const NAV_ICONS = {
  "quick-start": Rocket,
  explore: Compass,
  skills: Sparkles,
  plugins: Plug,
  about: Info,
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
    <aside className="glass-sidebar flex h-full w-64 shrink-0 flex-col border-r">
      <div className="flex items-center gap-3 px-5 py-5">
        <AppIcon size="sm" className="shadow-sm" />
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold tracking-tight">
            ZWitch
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
                  ? "glass-surface font-medium text-foreground ring-1 ring-border/60"
                  : "text-muted-foreground hover:bg-card/45 hover:text-foreground",
              )}
            >
              <Icon className="h-4 w-4 shrink-0" />
              {item.label}
            </button>
          );
        })}
      </nav>

      <div className="p-3">
        <div className="glass-surface flex items-center gap-3 rounded-xl p-2.5 ring-1 ring-border/60">
          <UserAvatar auth={auth} />
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium">
              {auth.user_name ?? "未命名用户"}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              {auth.department ?? auth.title ?? "—"}
            </p>
          </div>
          {onLogout ? (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="shrink-0 text-muted-foreground hover:text-foreground"
              title="退出登录"
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
