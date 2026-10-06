import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MouseEvent } from "react";
import { displayName, initials, statusClass, statusLabel } from "../utils/format";
import type { ConnectionSnapshot, SessionView, ThemeName } from "../types";

interface Props {
  connection: ConnectionSnapshot;
  session: SessionView;
  theme: ThemeName;
  desktop: boolean;
  onToggleTheme: () => void;
  onOpenServices: () => void;
  onLogout: () => void;
  onSignIn: () => void;
  onAbout: () => void;
}

export function TitleBar({
  connection,
  session,
  theme,
  desktop,
  onToggleTheme,
  onOpenServices,
  onLogout,
  onSignIn,
  onAbout,
}: Props) {
  const drag = (event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  };

  const name = displayName(session.user);
  const showAccount = session.authenticated && session.user;

  return (
    <header
      className="titlebar"
      onMouseDown={drag}
      onDoubleClick={() => void getCurrentWindow().toggleMaximize()}
    >
      <div className="lights">
        <button className="light close" aria-label="Close" onClick={() => void getCurrentWindow().close()}>
          <span>×</span>
        </button>
        <button className="light min" aria-label="Minimize" onClick={() => void getCurrentWindow().minimize()}>
          <span>–</span>
        </button>
        <button className="light max" aria-label="Maximize" onClick={() => void getCurrentWindow().toggleMaximize()}>
          <span>+</span>
        </button>
      </div>
      {desktop ? (
        <div className={`status ${statusClass(connection.state)}`} title={connection.detail}>
          <i />
          {statusLabel(connection.state)}
        </div>
      ) : null}
      <div className="title-spacer" data-tauri-drag-region />
      {desktop ? (
        <>
          <button className="icon-btn" aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"} onClick={onToggleTheme}>
            {theme === "dark" ? "light" : "dark"}
          </button>
          <button className="text-btn" aria-label="MCP servers" onClick={onOpenServices}>
            mcp
          </button>
        </>
      ) : null}
      {showAccount ? (
        <div className="account">
          <button className="logout" onClick={onLogout}>
            logout
          </button>
          <span className="bar">|</span>
          <button className="who icon-btn" onClick={onAbout} aria-label="About Unit Agent">
            <span>{name}</span>
            {session.source === "local" ? <span className="local">local</span> : null}
            {session.user?.picture ? (
              <img className="avatar" src={session.user.picture} alt="" />
            ) : (
              <span className="avatar-fallback" aria-hidden>
                {initials(name)}
              </span>
            )}
          </button>
        </div>
      ) : desktop && (connection.state === "ONLINE" || connection.state === "DEGRADED") ? (
        <button className="text-btn" onClick={onSignIn}>
          sign in
        </button>
      ) : null}
    </header>
  );
}
