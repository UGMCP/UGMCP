export type Phase = "boot" | "login" | "desktop" | "expired" | "fatal";
export type ThemeName = "dark" | "light";

export interface PublicUser {
  id: string;
  email: string;
  name?: string | null;
  username?: string | null;
  nickname?: string | null;
  picture?: string | null;
  role?: string | null;
}

export interface SessionView {
  authenticated: boolean;
  expired: boolean;
  source: string;
  user: PublicUser | null;
  issuedAt?: number | null;
  expiresAt?: number | null;
}

export interface HostIdentity {
  computerName: string;
  serverName: string;
  network: string;
}

export interface ConnectionSnapshot {
  state: string;
  internet: string;
  prysel: string;
  checkedAt: number | null;
  detail: string;
}

export interface WindowSettings {
  width: number;
  height: number;
  x: number | null;
  y: number | null;
  maximized: boolean;
}

export interface SavedWorkspace {
  id: string;
  title: string;
  cwd: string;
  shell: string;
  createdAt: number;
}

export interface Settings {
  theme: ThemeName;
  debug: boolean;
  fontSize: number;
  shell: string | null;
  defaultCwd: string | null;
  restoreWorkspaces: boolean;
  workspaces: SavedWorkspace[];
  activeWorkspace: string | null;
  window: WindowSettings;
  mcpServers: Array<{
    id: string;
    name: string;
    command: string;
    args: string[];
    autoStart: boolean;
  }>;
}

export interface TerminalInfo {
  id: string;
  title: string;
  cwd: string;
  shell: string;
  pid: number | null;
  running: boolean;
  createdAt: number;
  exitCode: number | null;
}

export interface TerminalSnapshot {
  info: TerminalInfo;
  output: string;
  seq: number;
}

export interface WriteResult {
  written: boolean;
  confirmation: string | null;
}

export type TerminalEvent =
  | { type: "output"; id: string; seq: number; data: string }
  | { type: "exit"; id: string; code: number | null }
  | { type: "cwd"; id: string; cwd: string }
  | { type: "confirm"; id: string; command: string };

export interface McpToolInfo {
  name: string;
  description: string;
}

export interface McpServerInfo {
  id: string;
  name: string;
  command: string;
  args: string[];
  status: string;
  tools: McpToolInfo[];
  message: string;
  running: boolean;
}

export interface AgentStatus {
  name: string;
  available: boolean;
  detail: string;
}

export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  arch: string;
  mode: string;
  config: {
    authUrl: string;
    apiUrl: string;
    appId: string;
    secretConfigured: boolean;
    redirectUri: string;
    callbackPort: number;
    agentConfigured: boolean;
  };
  shortcuts: Array<{ keys: string; action: string }>;
}

export interface DebugInfo {
  version: string;
  connection: ConnectionSnapshot;
  authSource: string;
  authenticated: boolean;
  shell: string;
  terminals: TerminalInfo[];
  mcp: McpServerInfo[];
  appId: string;
  secretConfigured: boolean;
}

export interface AppError {
  code: string;
  message: string;
}

export const emptySession = (): SessionView => ({
  authenticated: false,
  expired: false,
  source: "none",
  user: null,
});

export const emptyConnection = (): ConnectionSnapshot => ({
  state: "CHECKING",
  internet: "CHECKING",
  prysel: "UNKNOWN",
  checkedAt: null,
  detail: "Checking connectivity",
});
