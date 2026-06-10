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
  supported: boolean;
  installed: boolean;
  install_shell: string;
  quick_start_doc_url: string;
  config_enabled: boolean;
  has_grayscale?: boolean;
}

export type AppPage =
  | "quick-start"
  | "explore"
  | "skills"
  | "plugins"
  | "usage"
  | "about";

export interface UsageTotals {
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
  cost_usd: number;
  requests: number;
}

export interface ModelUsage extends UsageTotals {
  model: string;
}

export interface DailyUsage extends UsageTotals {
  date: string;
}

export interface UsageSummary {
  total: UsageTotals;
  models: ModelUsage[];
  calendar: DailyUsage[];
  active_days: number;
  first_recorded_at: string | null;
  last_recorded_at: string | null;
  today: string;
}

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

export interface UpdateDownloadProgress {
  downloaded: number;
  total: number | null;
}

export interface DownloadedUpdateInfo {
  ready: boolean;
  version: string | null;
  notes: string | null;
  deferred: boolean;
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
