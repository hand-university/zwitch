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
    <div className="flex h-full overflow-hidden bg-canvas text-foreground">
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
  );
}
