import { useCallback, useEffect, useState } from "react";
import {
  applyConfigInjection,
  getAuthState,
  getCliToolsStatus,
  getProxyEnabled,
  logout,
  onAuthChanged,
  onLoginFailed,
  openLoginWindow,
  refreshUserProfile,
  setProxyEnabled,
  setToolSwitch,
} from "@/lib/api";
import { AppLayout } from "@/components/layout/AppLayout";
import { Header } from "@/components/layout/Header";
import { LoginPage } from "@/pages/LoginPage";
import { QuickStartPage } from "@/pages/QuickStartPage";
import { isDev } from "@/config/env";
import type { AppPage, AuthState, CliToolStatus, NavItem } from "@/types";

const NAV_ITEMS: NavItem[] = [{ id: "quick-start", label: "快速开始" }];

const PAGE_META: Record<AppPage, { title: string; description: string }> = {
  "quick-start": {
    title: "快速开始",
    description: "管理 AI CLI 工具的代理配置",
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
  const [proxyEnabled, setProxyEnabledState] = useState(false);
  const [activePage, setActivePage] = useState<AppPage>("quick-start");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshTools = useCallback(async () => {
    const [cliTools, enabled] = await Promise.all([
      getCliToolsStatus(),
      getProxyEnabled(),
    ]);
    setTools(cliTools);
    setProxyEnabledState(enabled);
  }, []);

  const refresh = useCallback(async () => {
    setBusy(true);
    try {
      const authState = await getAuthState();
      setAuth(authState);
      if (authState.is_logged_in) {
        await refreshTools();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refreshTools]);

  const refreshProfile = useCallback(async () => {
    setBusy(true);
    try {
      const authState = await refreshUserProfile();
      setAuth(authState);
      if (authState.is_logged_in) {
        // 续期成功，清除可能存在的旧错误
        setError(null);
        await refreshTools();
      }
      // 若已登出（设备授权失效），保留后端通过 login-failed 推送的提示
    } catch (e) {
      setError(String(e));
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
        setError(String(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [refreshProfile]);

  useEffect(() => {
    const unlistenAuth = onAuthChanged((state) => {
      setAuth(state);
      setError(null);
      refreshTools().catch((e) => setError(String(e)));
    });

    const unlistenFail = onLoginFailed((message) => {
      setError(message);
    });

    return () => {
      unlistenAuth.then((fn) => fn());
      unlistenFail.then((fn) => fn());
    };
  }, [refreshTools]);

  const handleLogin = async () => {
    setError(null);
    try {
      await openLoginWindow();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleLogout = async () => {
    setBusy(true);
    try {
      await logout();
      setAuth(defaultAuth);
      setTools([]);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleToolSwitch = async (toolId: string, enabled: boolean) => {
    setBusy(true);
    try {
      await setToolSwitch(toolId, enabled);
      await applyConfigInjection();
      await refreshTools();
    } catch (e) {
      setError(String(e));
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
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center text-muted-foreground">
        加载中...
      </div>
    );
  }

  if (!auth.is_logged_in) {
    return <LoginPage error={error} onLogin={handleLogin} />;
  }

  const pageMeta = PAGE_META[activePage];

  return (
    <AppLayout
      auth={auth}
      activePage={activePage}
      navItems={NAV_ITEMS}
      onNavigate={setActivePage}
      onLogout={isDev ? handleLogout : undefined}
    >
      <Header title={pageMeta.title} description={pageMeta.description} />
      {error ? (
        <div className="mx-6 mt-4 shrink-0 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">
          {error}
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {activePage === "quick-start" ? (
          <QuickStartPage
            tools={tools}
            busy={busy}
            proxyEnabled={proxyEnabled}
            onRefresh={refresh}
            onToolSwitch={handleToolSwitch}
            onProxyToggle={handleProxyToggle}
          />
        ) : null}
      </div>
    </AppLayout>
  );
}
