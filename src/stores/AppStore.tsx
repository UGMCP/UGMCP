import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, asAppError } from "../services/api";
import type {
  AiActivityEntry,
  AiClient,
  AiConfirmRequest,
  ConnectionSnapshot,
  ControlEvent,
  Phase,
  SessionView,
  Settings,
  TerminalInfo,
  ThemeName,
} from "../types";
import { emptyConnection, emptySession } from "../types";
import { resolvePhase } from "../utils/startup";

interface Store {
  phase: Phase;
  fatalMessage: string | null;
  authBusy: boolean;
  authError: string | null;
  session: SessionView;
  connection: ConnectionSnapshot;
  settings: Settings | null;
  workspaces: TerminalInfo[];
  activeId: string | null;
  generations: Record<string, number>;
  servicesOpen: boolean;
  aboutOpen: boolean;
  aiOpen: boolean;
  aiClients: AiClient[];
  aiActivity: AiActivityEntry[];
  aiStopped: boolean;
  aiConfirm: AiConfirmRequest | null;
  notice: string | null;
  signIn: () => void;
  cancelSignIn: () => void;
  workOffline: () => void;
  logout: () => void;
  retryBoot: () => void;
  createWorkspace: () => void;
  focusWorkspace: (id: string) => void;
  noteCwd: (id: string, cwd: string) => void;
  removeWorkspace: (id: string) => void;
  noteRestart: (info: TerminalInfo) => void;
  toggleTheme: () => void;
  toggleServices: () => void;
  toggleAbout: () => void;
  toggleAi: () => void;
  confirmAi: (allow: boolean) => void;
  stopAi: () => void;
  resumeAi: () => void;
  updateSettings: (patch: Partial<Settings>) => void;
  reportError: (message: string) => void;
}

const Ctx = createContext<Store | null>(null);

const fallbackSettings = (): Settings => ({
  theme: "dark",
  debug: false,
  fontSize: 13,
  shell: null,
  defaultCwd: null,
  restoreWorkspaces: true,
  workspaces: [],
  activeWorkspace: null,
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
  mcpServers: [],
  aiMcp: true,
  aiPermission: "confirm",
  aiScreenshots: false,
  aiClipboard: false,
});

function applyTheme(theme: ThemeName) {
  document.documentElement.setAttribute("data-theme", theme);
  try {
    localStorage.setItem("unit-agent-theme", theme);
  } catch {
    /* private mode */
  }
}

export function AppStore({ children }: { children: ReactNode }) {
  const [phase, setPhase] = useState<Phase>("boot");
  const [fatalMessage, setFatalMessage] = useState<string | null>(null);
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [session, setSession] = useState<SessionView>(emptySession());
  const [connection, setConnection] = useState<ConnectionSnapshot>(emptyConnection());
  const [settings, setSettings] = useState<Settings | null>(null);
  const [workspaces, setWorkspaces] = useState<TerminalInfo[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [generations, setGenerations] = useState<Record<string, number>>({});
  const [servicesOpen, setServicesOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [aiOpen, setAiOpen] = useState(false);
  const [aiClients, setAiClients] = useState<AiClient[]>([]);
  const [aiActivity, setAiActivity] = useState<AiActivityEntry[]>([]);
  const [aiStopped, setAiStopped] = useState(false);
  const [aiConfirm, setAiConfirm] = useState<AiConfirmRequest | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const reportError = useCallback((message: string) => {
    setNotice(message);
  }, []);

  const loadWorkspaces = useCallback(async (current: Settings) => {
    let live = await api.terminalList();
    if (live.length === 0 && current.restoreWorkspaces && current.workspaces.length > 0) {
      for (const workspace of current.workspaces) {
        try {
          await api.terminalCreate({
            id: workspace.id,
            title: workspace.title,
            cwd: workspace.cwd,
            shell: workspace.shell,
          });
        } catch (error) {
          reportError(asAppError(error).message);
        }
      }
      live = await api.terminalList();
    }
    setWorkspaces(live);
    setActiveId((prev) => prev ?? current.activeWorkspace ?? live[0]?.id ?? null);
  }, [reportError]);

  const boot = useCallback(async () => {
    setPhase("boot");
    setFatalMessage(null);
    try {
      const loaded = await api.settingsGet();
      applyTheme(loaded.theme);
      setSettings(loaded);
      const [restored, snapshot] = await Promise.all([api.sessionRestore(), api.connectionSnapshot()]);
      setSession(restored);
      setConnection(snapshot);
      const next = resolvePhase({ fatal: false, session: restored, offlineChosen: false });
      setPhase(next);
      if (next === "desktop") await loadWorkspaces(loaded);
    } catch (error) {
      setFatalMessage(asAppError(error).message);
      setPhase("fatal");
    }
  }, [loadWorkspaces]);

  useEffect(() => {
    void boot();
  }, [boot]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<ConnectionSnapshot>("connection-changed", (event) => {
      if (event.payload) setConnection(event.payload);
    }).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<ControlEvent>("control-event", (event) => {
      const payload = event.payload;
      if (!payload) return;
      if (payload.type === "workspace") {
        setWorkspaces((current) => {
          const existing = current.find((item) => item.id === payload.info.id);
          if (!existing) return [...current, payload.info];
          return current.map((item) => (item.id === payload.info.id ? payload.info : item));
        });
        if (payload.focus) setActiveId(payload.info.id);
      } else if (payload.type === "confirm") {
        setAiConfirm({
          id: payload.id,
          client: payload.client,
          action: payload.action,
          workspace: payload.workspace,
          command: payload.command,
        });
        setAiOpen(true);
      } else if (payload.type === "activity") {
        setAiActivity((current) => [payload.entry, ...current].slice(0, 40));
        if (payload.entry.status === "notice") setNotice(payload.entry.action);
      } else if (payload.type === "status") {
        void api.aiStatus().then((status) => {
          setAiClients(status.clients ?? []);
          setAiStopped(Boolean(status.emergencyStop));
        }).catch(() => undefined);
      }
    }).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 4200);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    let timer: number | undefined;
    const save = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        void (async () => {
          try {
            const win = getCurrentWindow();
            const factor = await win.scaleFactor();
            const size = await win.innerSize();
            const pos = await win.outerPosition();
            const maximized = await win.isMaximized();
            const next = await api.settingsUpdate({
              window: {
                width: size.width / factor,
                height: size.height / factor,
                x: pos.x / factor,
                y: pos.y / factor,
                maximized,
              },
            });
            setSettings(next);
          } catch {
            /* window metrics are optional */
          }
        })();
      }, 400);
    };
    let unlisten: Array<() => void> = [];
    void (async () => {
      const win = getCurrentWindow();
      unlisten = [await win.onResized(save), await win.onMoved(save)];
    })();
    return () => {
      window.clearTimeout(timer);
      for (const stop of unlisten) stop();
    };
  }, []);

  const enterDesktop = useCallback(async () => {
    setPhase("desktop");
    const current = settings ?? (await api.settingsGet().catch(() => fallbackSettings()));
    setSettings(current);
    await loadWorkspaces(current);
  }, [loadWorkspaces, settings]);

  const signIn = useCallback(() => {
    setAuthBusy(true);
    setAuthError(null);
    void api
      .authLogin()
      .then(async (next) => {
        setSession(next);
        await enterDesktop();
      })
      .catch((error: unknown) => {
        const parsed = asAppError(error);
        if (parsed.code !== "AUTH_CANCELLED") setAuthError(parsed.message);
      })
      .finally(() => setAuthBusy(false));
  }, [enterDesktop]);

  const cancelSignIn = useCallback(() => {
    void api.authCancel();
    setAuthBusy(false);
  }, []);

  const workOffline = useCallback(() => {
    setAuthError(null);
    setAuthBusy(false);
    void enterDesktop();
  }, [enterDesktop]);

  const logout = useCallback(() => {
    void api.authLogout().then((next) => {
      setSession(next);
      setPhase("login");
      setServicesOpen(false);
    }).catch((error: unknown) => reportError(asAppError(error).message));
  }, [reportError]);

  const createWorkspace = useCallback(() => {
    const cwd = settings?.defaultCwd || undefined;
    const shell = settings?.shell || undefined;
    void api
      .terminalCreate({ title: "Terminal", cwd, shell })
      .then((info) => {
        setWorkspaces((current) => [...current, info]);
        setActiveId(info.id);
        void api.settingsUpdate({ activeWorkspace: info.id });
      })
      .catch((error: unknown) => reportError(asAppError(error).message));
  }, [reportError, settings]);

  const focusWorkspace = useCallback((id: string) => {
    setActiveId(id);
    void api.settingsUpdate({ activeWorkspace: id }).then(setSettings).catch(() => undefined);
  }, []);

  const noteCwd = useCallback((id: string, cwd: string) => {
    setWorkspaces((current) => current.map((workspace) => (workspace.id === id ? { ...workspace, cwd } : workspace)));
  }, []);

  const removeWorkspace = useCallback((id: string) => {
    setWorkspaces((current) => current.filter((workspace) => workspace.id !== id));
    setActiveId((current) => (current === id ? null : current));
  }, []);

  const noteRestart = useCallback((info: TerminalInfo) => {
    setGenerations((current) => ({ ...current, [info.id]: (current[info.id] ?? 0) + 1 }));
    setWorkspaces((current) => current.map((workspace) => (workspace.id === info.id ? info : workspace)));
  }, []);

  const updateSettings = useCallback((patch: Partial<Settings>) => {
    void api.settingsUpdate(patch).then((next) => {
      setSettings(next);
      if (patch.theme) applyTheme(next.theme);
    }).catch((error: unknown) => reportError(asAppError(error).message));
  }, [reportError]);

  const confirmAi = useCallback((allow: boolean) => {
    setAiConfirm((current) => {
      if (current) void api.aiConfirm(current.id, allow).catch(() => undefined);
      return null;
    });
  }, []);

  const stopAi = useCallback(() => {
    void api.aiEmergencyStop().then(() => {
      setAiStopped(true);
      setAiClients([]);
      setNotice("AI control disabled. Terminals keep running.");
    }).catch((error: unknown) => reportError(asAppError(error).message));
  }, [reportError]);

  const resumeAi = useCallback(() => {
    void api.aiResumeControl().then(() => {
      setAiStopped(false);
      setNotice("AI control is available again.");
    }).catch((error: unknown) => reportError(asAppError(error).message));
  }, [reportError]);

  const toggleTheme = useCallback(() => {
    const next: ThemeName = (settings?.theme ?? "dark") === "dark" ? "light" : "dark";
    applyTheme(next);
    updateSettings({ theme: next });
  }, [settings, updateSettings]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || !event.shiftKey) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest(".panel input, .panel textarea, .login")) return;
      const key = event.key.toLowerCase();
      if (key === "t" || key === "n") {
        event.preventDefault();
        if (phase === "desktop") createWorkspace();
      } else if (key === "w") {
        event.preventDefault();
        if (activeId) {
          void api.terminalClose(activeId, false).then(() => removeWorkspace(activeId)).catch((error: unknown) => {
            const parsed = asAppError(error);
            if (parsed.code !== "NEEDS_CONFIRM") reportError(parsed.message);
            else reportError(parsed.message);
          });
        }
      } else if (key === "l") {
        event.preventDefault();
        toggleTheme();
      } else if (key === "m") {
        event.preventDefault();
        setServicesOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activeId, createWorkspace, phase, removeWorkspace, reportError, toggleTheme]);

  const value = useMemo<Store>(() => ({
    phase,
    fatalMessage,
    authBusy,
    authError,
    session,
    connection,
    settings,
    workspaces,
    activeId,
    generations,
    servicesOpen,
    aboutOpen,
    aiOpen,
    aiClients,
    aiActivity,
    aiStopped,
    aiConfirm,
    notice,
    signIn,
    cancelSignIn,
    workOffline,
    logout,
    retryBoot: () => void boot(),
    createWorkspace,
    focusWorkspace,
    noteCwd,
    removeWorkspace,
    noteRestart,
    toggleTheme,
    toggleServices: () => setServicesOpen((open) => !open),
    toggleAbout: () => setAboutOpen((open) => !open),
    toggleAi: () => {
      setAiOpen((open) => !open);
      void api.aiStatus().then((status) => {
        setAiClients(status.clients ?? []);
        setAiStopped(Boolean(status.emergencyStop));
      }).catch(() => undefined);
      void api.aiActivity().then(setAiActivity).catch(() => undefined);
    },
    confirmAi,
    stopAi,
    resumeAi,
    updateSettings,
    reportError,
  }), [
    phase, fatalMessage, authBusy, authError, session, connection, settings, workspaces, activeId,
    generations, servicesOpen, aboutOpen, aiOpen, aiClients, aiActivity, aiStopped, aiConfirm, notice,
    signIn, cancelSignIn, workOffline, logout, boot, createWorkspace, focusWorkspace, noteCwd,
    removeWorkspace, noteRestart, toggleTheme, updateSettings, reportError, confirmAi, stopAi, resumeAi,
  ]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAppStore(): Store {
  const value = useContext(Ctx);
  if (!value) throw new Error("AppStore missing");
  return value;
}
