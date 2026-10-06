import { invoke } from "@tauri-apps/api/core";
import type {
  AgentStatus,
  AppError,
  AppInfo,
  ConnectionSnapshot,
  DebugInfo,
  McpServerInfo,
  SessionView,
  Settings,
  TerminalInfo,
  TerminalSnapshot,
  WriteResult,
} from "../types";

export function asAppError(error: unknown): AppError {
  if (typeof error === "string") {
    try {
      const parsed = JSON.parse(error) as Partial<AppError>;
      if (parsed && typeof parsed.message === "string") {
        return { code: parsed.code ?? "COMMAND_FAILED", message: parsed.message };
      }
    } catch {
      return { code: "COMMAND_FAILED", message: error };
    }
    return { code: "COMMAND_FAILED", message: error };
  }
  if (error && typeof error === "object" && "message" in error) {
    const record = error as Partial<AppError>;
    return {
      code: record.code ?? "COMMAND_FAILED",
      message: record.message ?? "Something went wrong.",
    };
  }
  return { code: "COMMAND_FAILED", message: "Something went wrong." };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}

export const api = {
  sessionRestore: () => call<SessionView>("session_restore"),
  authLogin: () => call<SessionView>("auth_login"),
  authCancel: () => call<void>("auth_cancel"),
  authLogout: () => call<SessionView>("auth_logout"),
  connectionSnapshot: () => call<ConnectionSnapshot>("connection_snapshot"),
  connectionRefresh: () => call<ConnectionSnapshot>("connection_refresh"),
  terminalCreate: (request: { id?: string; title?: string; cwd?: string; shell?: string }) =>
    call<TerminalInfo>("terminal_create", { request }),
  terminalWrite: (id: string, data: string) => call<WriteResult>("terminal_write", { id, data }),
  terminalResize: (id: string, cols: number, rows: number) => call<void>("terminal_resize", { id, cols, rows }),
  terminalClose: (id: string, force: boolean) => call<void>("terminal_close", { id, force }),
  terminalRestart: (id: string) => call<TerminalInfo>("terminal_restart", { id }),
  terminalList: () => call<TerminalInfo[]>("terminal_list"),
  terminalSnapshot: (id: string) => call<TerminalSnapshot>("terminal_snapshot", { id }),
  terminalConfirm: (id: string, approve: boolean) => call<void>("terminal_confirm", { id, approve }),
  terminalSetCwd: (id: string, cwd: string) => call<void>("terminal_set_cwd", { id, cwd }),
  settingsGet: () => call<Settings>("settings_get"),
  settingsUpdate: (patch: Partial<Settings>) => call<Settings>("settings_update", { patch }),
  mcpList: () => call<McpServerInfo[]>("mcp_list"),
  mcpConnect: (request: { name: string; command: string; args: string[]; env: Record<string, string> }) =>
    call<McpServerInfo>("mcp_connect", { request }),
  mcpStart: (id: string) => call<McpServerInfo>("mcp_start", { id }),
  mcpDisconnect: (id: string) => call<void>("mcp_disconnect", { id }),
  mcpForget: (id: string) => call<void>("mcp_forget", { id }),
  mcpCallTool: (request: {
    id: string;
    name: string;
    arguments: Record<string, unknown>;
    confirmed: boolean;
    acknowledgedDanger: boolean;
  }) => call<unknown>("mcp_call_tool", { request }),
  appInfo: () => call<AppInfo>("app_info"),
  debugInfo: () => call<DebugInfo>("debug_info"),
  agentStatus: () => call<{ local: AgentStatus; prysel: AgentStatus }>("agent_status"),
};
