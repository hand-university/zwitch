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
  switch_enabled: boolean;
  install_url: string;
}

export type AppPage = "quick-start";

export interface NavItem {
  id: AppPage;
  label: string;
}
