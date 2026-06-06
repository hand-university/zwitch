export interface AuthState {
  is_logged_in: boolean;
  api_key: string | null;
  user_name: string | null;
  avatar: string | null;
  department: string | null;
  title: string | null;
}

export interface CliToolStatus {
  id: string;
  name: string;
  installed: boolean;
  install_url: string;
}

export type AppPage = "quick-start" | "explore" | "skills" | "plugins" | "about";

export interface AppInfo {
  name: string;
  version: string;
  identifier: string;
}

export interface UpdateCheckResult {
  available: boolean;
  currentVersion: string;
  version: string | null;
  notes: string | null;
  date: string | null;
}

export interface NavItem {
  id: AppPage;
  label: string;
}

export interface MarketplaceItem {
  id: string;
  remote_id: number | null;
  name: string;
  platform: "claude" | "codex";
  item_type: "skill" | "plugin";
  description: string | null;
  version: string | null;
  enabled: boolean;
  remote_available: boolean;
  origin: "remote" | "local" | "both";
  installed: boolean;
  updated_at: string | null;
}

export interface ExploreItem {
  id: string;
  remote_id: number;
  name: string;
  platform: "claude" | "codex";
  item_type: "skill" | "plugin";
  description: string | null;
  version: string | null;
  updated_at: string | null;
  installed: boolean;
  enabled: boolean;
}

export interface MarketplaceSyncResult {
  synced: number;
  updated: number;
  removed_from_remote: number;
  discovered_local: number;
}
