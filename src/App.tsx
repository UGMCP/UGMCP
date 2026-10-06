import { useEffect, useState } from "react";
import { Desktop } from "./components/Desktop";
import { LoginScreen } from "./components/LoginScreen";
import { ServicesPanel } from "./components/ServicesPanel";
import { TitleBar } from "./components/TitleBar";
import { api } from "./services/api";
import { AppStore, useAppStore } from "./stores/AppStore";
import type { AppInfo, DebugInfo } from "./types";

function Shell() {
  const app = useAppStore();
  const theme = app.settings?.theme ?? "dark";
  const [about, setAbout] = useState<AppInfo | null>(null);
  const [debug, setDebug] = useState<DebugInfo | null>(null);

  useEffect(() => {
    if (!app.aboutOpen) return;
    void api.appInfo().then(setAbout).catch(() => undefined);
  }, [app.aboutOpen]);

  useEffect(() => {
    if (!app.settings?.debug || app.phase !== "desktop") return;
    void api.debugInfo().then(setDebug).catch(() => undefined);
  }, [app.settings?.debug, app.phase, app.connection.state, app.workspaces.length]);

  return (
    <div className="app">
      <TitleBar
        connection={app.connection}
        session={app.session}
        theme={theme}
        desktop={app.phase === "desktop"}
        onToggleTheme={app.toggleTheme}
        onOpenServices={app.toggleServices}
        onLogout={app.logout}
        onSignIn={app.signIn}
        onAbout={app.toggleAbout}
      />
      <main className="stage">
        {app.phase === "boot" ? (
          <section className="center-copy">
            <h1 className="wordmark">Unit Agent</h1>
          </section>
        ) : null}
        {app.phase === "login" ? (
          <LoginScreen
            title="Unit Agent"
            primary="Sign in with Prysel Auth"
            secondary="Work Offline"
            busy={app.authBusy}
            error={app.authError}
            onPrimary={app.signIn}
            onSecondary={app.workOffline}
            onCancel={app.cancelSignIn}
          />
        ) : null}
        {app.phase === "expired" ? (
          <LoginScreen
            title="Unit Agent"
            primary="Sign in again"
            secondary="Continue Offline"
            busy={app.authBusy}
            error={app.authError ?? "Session expired"}
            onPrimary={app.signIn}
            onSecondary={app.workOffline}
            onCancel={app.cancelSignIn}
          />
        ) : null}
        {app.phase === "fatal" ? (
          <LoginScreen
            title="Unit Agent"
            primary="Retry"
            secondary="Work Offline"
            error={app.fatalMessage ?? "Something went wrong."}
            onPrimary={app.retryBoot}
            onSecondary={app.workOffline}
          />
        ) : null}
        {app.phase === "desktop" ? (
          <Desktop
            workspaces={app.workspaces}
            activeId={app.activeId}
            fontSize={app.settings?.fontSize ?? 13}
            theme={theme}
            generations={app.generations}
            onCreate={app.createWorkspace}
            onFocus={app.focusWorkspace}
            onCwd={app.noteCwd}
            onClosed={app.removeWorkspace}
            onRestarted={app.noteRestart}
            onError={app.reportError}
          />
        ) : null}
        {app.servicesOpen && app.settings ? (
          <ServicesPanel
            connection={app.connection}
            settings={app.settings}
            onClose={app.toggleServices}
            onSettings={app.updateSettings}
            onError={app.reportError}
          />
        ) : null}
        {app.aboutOpen ? (
          <>
            <button className="panel-backdrop" aria-label="Close about" onClick={app.toggleAbout} />
            <aside className="panel" role="dialog" aria-label="About Unit Agent">
              <h2>About Unit Agent</h2>
              <p>
                {about?.name ?? "Unit Agent"} {about?.version ?? "0.1.0"} runs on {about?.platform ?? "this computer"}.
                Terminal sessions stay on this machine. Prysel authentication is optional and uses auth.prysel.com.
              </p>
              <p>License: Proprietary — Prysel. All rights reserved.</p>
              {about ? (
                <p>
                  Callback {about.config.redirectUri}. Application id {about.config.appId || "not configured"}.
                </p>
              ) : null}
            </aside>
          </>
        ) : null}
        {app.notice ? <div className="toast" role="status">{app.notice}</div> : null}
        {debug && app.settings?.debug ? (
          <pre className="debug">
            {`version ${debug.version}
auth ${debug.authSource} authenticated=${debug.authenticated}
shell ${debug.shell}
internet ${debug.connection.internet} prysel ${debug.connection.prysel}
terminals ${debug.terminals.map((item) => `${item.id} pid=${item.pid ?? "-"} ${item.running ? "running" : "exited"}`).join("\n")}`}
          </pre>
        ) : null}
      </main>
    </div>
  );
}

export function App() {
  return (
    <AppStore>
      <Shell />
    </AppStore>
  );
}
