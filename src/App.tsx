import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ExplorePage } from "@/pages/ExplorePage";
import { Cloud, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  deleteMarketplaceItem,
  getAuthState,
  getCliToolsStatus,
  getExploreItems,
  getMarketplaceItems,
  installMarketplaceItem,
  getProxyEnabled,
  logout,
  onAuthChanged,
  onLoginFailed,
  openLoginWindow,
  refreshUserProfile,
  scanLocalMarketplace,
  setMarketplaceItemEnabled,
  setProxyEnabled,
  syncMarketplace,
} from "@/lib/api";
import { AppLayout } from "@/components/layout/AppLayout";
import { Header } from "@/components/layout/Header";
import { message, MessageHost } from "@/components/ui/message";
import { LoginPage } from "@/pages/LoginPage";
import { MarketplacePage } from "@/pages/MarketplacePage";
import { AboutPage } from "@/pages/AboutPage";
import { QuickStartPage } from "@/pages/QuickStartPage";
import { isDev } from "@/config/env";
import type {
  AppPage,
  AuthState,
  CliToolStatus,
  ExploreItem,
  MarketplaceItem,
  NavItem,
} from "@/types";

const NAV_ITEMS: NavItem[] = [
  { id: "quick-start", label: "快速开始" },
  { id: "explore", label: "探索" },
  { id: "skills", label: "技能" },
  { id: "plugins", label: "插件" },
  { id: "about", label: "关于" },
];

const PAGE_META: Record<AppPage, { title: string; description: string }> = {
  "quick-start": {
    title: "快速开始",
    description: "一键开启代理，自动配置已安装的工具",
  },
  explore: {
    title: "探索",
    description: "浏览云端分配给你的技能与插件",
  },
  skills: {
    title: "技能",
    description: "管理已安装的技能",
  },
  plugins: {
    title: "插件",
    description: "管理已安装的插件",
  },
  about: {
    title: "关于",
    description: "应用信息与版本更新",
  },
};

const defaultAuth: AuthState = {
  is_logged_in: false,
  api_key: null,
  user_name: null,
  avatar: null,
  department: null,
  title: null,
};

export default function App() {
  const [auth, setAuth] = useState<AuthState>(defaultAuth);
  const [tools, setTools] = useState<CliToolStatus[]>([]);
  const [marketplaceItems, setMarketplaceItems] = useState<MarketplaceItem[]>([]);
  const [exploreItems, setExploreItems] = useState<ExploreItem[]>([]);
  const [proxyEnabled, setProxyEnabledState] = useState(false);
  const [activePage, setActivePage] = useState<AppPage>("quick-start");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);

  const refreshTools = useCallback(async () => {
    const [cliTools, enabled, items] = await Promise.all([
      getCliToolsStatus(),
      getProxyEnabled(),
      getMarketplaceItems(),
    ]);
    setTools(cliTools);
    setProxyEnabledState(enabled);
    setMarketplaceItems(items);
  }, []);

  const loadExploreItems = useCallback(async () => {
    const items = await getExploreItems();
    setExploreItems(items);
  }, []);

  const refresh = useCallback(async () => {
    setBusy(true);
    try {
      const authState = await getAuthState();
      setAuth(authState);
      if (authState.is_logged_in) {
        await refreshTools();
        if (activePage === "explore") {
          await loadExploreItems();
        }
      }
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  }, [refreshTools, loadExploreItems, activePage]);

  const refreshProfile = useCallback(async () => {
    setBusy(true);
    try {
      const authState = await refreshUserProfile();
      setAuth(authState);
      if (authState.is_logged_in) {
        await refreshTools();
      }
      // 若已登出（设备授权失效），保留后端通过 login-failed 推送的提示
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  }, [refreshTools]);

  useEffect(() => {
    (async () => {
      try {
        const authState = await getAuthState();
        setAuth(authState);
        if (authState.is_logged_in) {
          await refreshProfile();
        }
      } catch (e) {
        message.error(String(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [refreshProfile]);

  useEffect(() => {
    const unlistenAuth = onAuthChanged((state) => {
      setAuth(state);
      refreshTools().catch((e) => message.error(String(e)));
    });

    const unlistenFail = onLoginFailed((text) => {
      message.error(text);
    });

    return () => {
      unlistenAuth.then((fn) => fn());
      unlistenFail.then((fn) => fn());
    };
  }, [refreshTools]);

  useEffect(() => {
    if (!auth.is_logged_in || activePage !== "explore") {
      return;
    }
    setBusy(true);
    loadExploreItems()
      .catch((e) => message.error(String(e)))
      .finally(() => setBusy(false));
  }, [auth.is_logged_in, activePage, loadExploreItems]);

  const handleLogin = async () => {
    try {
      await openLoginWindow();
    } catch (e) {
      message.error(String(e));
    }
  };

  const handleLogout = async () => {
    setBusy(true);
    try {
      await logout();
      setAuth(defaultAuth);
      setTools([]);
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleProxyToggle = async (enabled: boolean) => {
    setBusy(true);
    try {
      await setProxyEnabled(enabled);
      await refreshTools();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleMarketplaceSync = async () => {
    setBusy(true);
    try {
      await syncMarketplace();
      await Promise.all([refreshTools(), loadExploreItems()]);
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const refreshMarketplace = useCallback(async () => {
    setBusy(true);
    try {
      await scanLocalMarketplace();
      await refreshTools();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  }, [refreshTools]);

  const handleMarketplaceToggle = async (
    item: MarketplaceItem,
    enabled: boolean,
  ) => {
    setBusy(true);
    try {
      await setMarketplaceItemEnabled(
        item.platform,
        item.item_type,
        item.name,
        enabled,
      );
      await refreshTools();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleExploreInstall = async (item: ExploreItem) => {
    setBusy(true);
    try {
      await installMarketplaceItem(item.platform, item.item_type, item.name);
      await Promise.all([refreshTools(), loadExploreItems()]);
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleMarketplaceDelete = async (item: MarketplaceItem) => {
    setBusy(true);
    try {
      await deleteMarketplaceItem(item.platform, item.item_type, item.name);
      setMarketplaceItems((prev) =>
        prev.filter(
          (entry) =>
            !(
              entry.platform === item.platform &&
              entry.item_type === item.item_type &&
              entry.name === item.name
            ),
        ),
      );
      await Promise.all([refreshTools(), loadExploreItems()]);
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const pageMeta = PAGE_META[activePage];

  const refreshExploreItems = useCallback(async () => {
    setBusy(true);
    try {
      await loadExploreItems();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  }, [loadExploreItems]);

  const headerActions =
    activePage === "quick-start" ? (
      <Button variant="outline" size="sm" onClick={refresh} disabled={busy}>
        <RefreshCw className={busy ? "animate-spin" : ""} />
        刷新
      </Button>
    ) : activePage === "skills" || activePage === "plugins" ? (
      <>
        <Button variant="outline" size="sm" onClick={refreshMarketplace} disabled={busy}>
          <RefreshCw className={busy ? "animate-spin" : ""} />
          刷新
        </Button>
        <Button size="sm" onClick={handleMarketplaceSync} disabled={busy}>
          <Cloud />
          一键同步市场
        </Button>
      </>
    ) : null;

  let content: ReactNode;

  if (loading) {
    content = (
      <div className="app-canvas flex min-h-screen items-center justify-center text-muted-foreground">
        加载中...
      </div>
    );
  } else if (!auth.is_logged_in) {
    content = <LoginPage onLogin={handleLogin} />;
  } else {
    content = (
      <AppLayout
        auth={auth}
        activePage={activePage}
        navItems={NAV_ITEMS}
        onNavigate={setActivePage}
        onLogout={isDev ? handleLogout : undefined}
      >
        <Header
          title={pageMeta.title}
          description={pageMeta.description}
          actions={headerActions}
        />
        <div className="min-h-0 flex-1 overflow-y-auto">
          {activePage === "quick-start" ? (
            <QuickStartPage
              tools={tools}
              busy={busy}
              proxyEnabled={proxyEnabled}
              onProxyToggle={handleProxyToggle}
            />
          ) : null}
          {activePage === "explore" ? (
            <ExplorePage
              items={exploreItems}
              busy={busy}
              onInstall={handleExploreInstall}
              onSearch={refreshExploreItems}
            />
          ) : null}
          {activePage === "skills" || activePage === "plugins" ? (
            <MarketplacePage
              itemType={activePage === "skills" ? "skill" : "plugin"}
              items={marketplaceItems}
              busy={busy}
              onToggle={handleMarketplaceToggle}
              onDelete={handleMarketplaceDelete}
            />
          ) : null}
          {activePage === "about" ? <AboutPage busy={busy} /> : null}
        </div>
      </AppLayout>
    );
  }

  return (
    <>
      <MessageHost />
      {content}
    </>
  );
}
