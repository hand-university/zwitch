import type { ReactNode } from "react";
import type { AuthState, AppPage, NavItem } from "@/types";
import { Sidebar } from "./Sidebar";

interface AppLayoutProps {
  auth: AuthState;
  activePage: AppPage;
  navItems: NavItem[];
  onNavigate: (page: AppPage) => void;
  onLogout?: () => void;
  children: ReactNode;
}

export function AppLayout({
  auth,
  activePage,
  navItems,
  onNavigate,
  onLogout,
  children,
}: AppLayoutProps) {
  return (
    <div className="app-canvas relative flex h-full overflow-hidden text-foreground">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 overflow-hidden"
      >
        <div className="absolute -left-16 -top-16 h-64 w-64 rounded-full bg-primary/[0.04] blur-3xl" />
        <div className="absolute -bottom-20 -right-16 h-72 w-72 rounded-full bg-emerald-500/[0.05] blur-3xl" />
      </div>

      <div className="relative flex h-full w-full overflow-hidden">
        <Sidebar
          auth={auth}
          items={navItems}
          activePage={activePage}
          onNavigate={onNavigate}
          onLogout={onLogout}
        />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {children}
        </main>
      </div>
    </div>
  );
}
