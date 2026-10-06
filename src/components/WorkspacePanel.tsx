import { useState } from "react";
import { api, asAppError } from "../services/api";
import type { TerminalInfo, ThemeName } from "../types";
import { shortPath } from "../utils/format";
import { TerminalView } from "./TerminalView";

interface Props {
  info: TerminalInfo;
  active: boolean;
  fontSize: number;
  theme: ThemeName;
  generation: number;
  onFocus: () => void;
  onCwd: (cwd: string) => void;
  onClosed: () => void;
  onRestarted: (info: TerminalInfo) => void;
  onError: (message: string) => void;
}

export function WorkspacePanel({
  info,
  active,
  fontSize,
  theme,
  generation,
  onFocus,
  onCwd,
  onClosed,
  onRestarted,
  onError,
}: Props) {
  const [confirm, setConfirm] = useState<string | null>(null);
  const [closeConfirm, setCloseConfirm] = useState(false);
  const [exited, setExited] = useState<number | null | false>(info.running ? false : info.exitCode);

  async function close(force: boolean) {
    try {
      await api.terminalClose(info.id, force);
      onClosed();
    } catch (error) {
      const parsed = asAppError(error);
      if (parsed.code === "NEEDS_CONFIRM") setCloseConfirm(true);
      else onError(parsed.message);
    }
  }

  return (
    <section
      className={`workspace${active ? " active" : ""}${confirm || closeConfirm ? " confirming" : ""}`}
      onMouseDown={onFocus}
    >
      <header className="workspace-meta">
        <span>{shortPath(info.cwd)}</span>
        <button className="close" aria-label={`Close ${info.title}`} onClick={() => void close(false)}>
          ×
        </button>
      </header>
      <TerminalView
        key={`${info.id}:${generation}`}
        id={info.id}
        fontSize={fontSize}
        theme={theme}
        onCwd={onCwd}
        onExit={(code) => setExited(code)}
        onConfirm={setConfirm}
      />
      {confirm ? (
        <div className="bar-overlay" role="alertdialog" aria-label="Confirm command">
          <p>
            Confirm destructive command <code>{confirm}</code>
          </p>
          <div className="bar-actions">
            <button
              className="ghost"
              onClick={() => {
                void api.terminalConfirm(info.id, false);
                setConfirm(null);
              }}
            >
              Cancel
            </button>
            <button
              className="danger-btn"
              onClick={() => {
                void api.terminalConfirm(info.id, true);
                setConfirm(null);
              }}
            >
              Run
            </button>
          </div>
        </div>
      ) : null}
      {closeConfirm ? (
        <div className="bar-overlay" role="alertdialog" aria-label="Close workspace">
          <p>Terminal process is still running. Close workspace?</p>
          <div className="bar-actions">
            <button className="ghost" onClick={() => setCloseConfirm(false)}>
              Cancel
            </button>
            <button className="danger-btn" onClick={() => void close(true)}>
              Close
            </button>
          </div>
        </div>
      ) : null}
      {exited !== false && !confirm && !closeConfirm ? (
        <div className="exit-overlay">
          <p>Process exited{exited === null ? "" : ` (${exited})`}</p>
          <div className="bar-actions">
            <button
              className="ghost"
              onClick={() => {
                void api.terminalRestart(info.id).then((next) => {
                  setExited(false);
                  onRestarted(next);
                }).catch((error: unknown) => onError(asAppError(error).message));
              }}
            >
              Restart terminal
            </button>
            <button className="ghost" onClick={() => void close(true)}>
              Close workspace
            </button>
          </div>
        </div>
      ) : null}
    </section>
  );
}
