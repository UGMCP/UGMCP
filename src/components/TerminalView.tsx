import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { listen } from "@tauri-apps/api/event";
import { memo, useEffect, useRef } from "react";
import "@xterm/xterm/css/xterm.css";
import { api } from "../services/api";
import type { TerminalEvent, ThemeName } from "../types";
import { osc7Path } from "../utils/format";
import { TERMINAL_FONT, terminalTheme } from "../utils/terminalTheme";

interface Props {
  id: string;
  fontSize: number;
  theme: ThemeName;
  onCwd: (cwd: string) => void;
  onExit: (code: number | null) => void;
  onConfirm: (command: string | null) => void;
}

export const TerminalView = memo(function TerminalView({ id, fontSize, theme, onCwd, onExit, onConfirm }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const callbacks = useRef({ onCwd, onExit, onConfirm });
  callbacks.current = { onCwd, onExit, onConfirm };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    const term = new Terminal({
      fontFamily: TERMINAL_FONT,
      fontSize,
      theme: terminalTheme(theme),
      cursorBlink: true,
      scrollback: 5000,
      convertEol: false,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    term.parser.registerOscHandler(7, (data) => {
      const path = osc7Path(data);
      if (path) {
        callbacks.current.onCwd(path);
        void api.terminalSetCwd(id, path);
      }
      return true;
    });
    term.onData((data) => {
      void api.terminalWrite(id, data);
    });
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown" || !event.ctrlKey || !event.shiftKey) return true;
      const key = event.key.toLowerCase();
      if (key === "c") {
        const selection = term.getSelection();
        if (selection) void navigator.clipboard.writeText(selection);
        return false;
      }
      if (key === "v") {
        void navigator.clipboard.readText().then((text) => {
          if (text) void api.terminalWrite(id, text);
        });
        return false;
      }
      if (key === "t" || key === "n" || key === "w" || key === "l" || key === "m") return false;
      return true;
    });

    const resize = () => {
      if (!host.isConnected || disposed) return;
      fit.fit();
      void api.terminalResize(id, term.cols, term.rows);
    };
    const observer = new ResizeObserver(() => resize());
    observer.observe(host);

    let unlisten: (() => void) | undefined;
    const buffered: Array<{ seq: number; data: string }> = [];
    let ready = false;
    let seq = 0;
    const apply = (chunk: { seq: number; data: string }) => {
      if (chunk.seq <= seq) return;
      seq = chunk.seq;
      term.write(chunk.data);
    };

    void (async () => {
      unlisten = await listen<TerminalEvent>("terminal-event", (event) => {
        const payload = event.payload;
        if (!payload || payload.id !== id) return;
        if (payload.type === "output") {
          if (!ready) buffered.push(payload);
          else apply(payload);
        } else if (payload.type === "exit") {
          callbacks.current.onExit(payload.code);
        } else if (payload.type === "cwd") {
          callbacks.current.onCwd(payload.cwd);
        } else if (payload.type === "confirm") {
          callbacks.current.onConfirm(payload.command);
        }
      });
      if (disposed) {
        unlisten?.();
        return;
      }
      const snapshot = await api.terminalSnapshot(id);
      if (disposed) return;
      if (snapshot.output) term.write(snapshot.output);
      seq = snapshot.seq;
      ready = true;
      for (const chunk of buffered) apply(chunk);
      buffered.length = 0;
      requestAnimationFrame(() => {
        resize();
        term.focus();
      });
    })();

    const paste = (event: MouseEvent) => {
      if (event.button !== 2) return;
      event.preventDefault();
      void navigator.clipboard.readText().then((text) => {
        if (text) void api.terminalWrite(id, text);
      });
    };
    host.addEventListener("contextmenu", paste);

    return () => {
      disposed = true;
      observer.disconnect();
      host.removeEventListener("contextmenu", paste);
      unlisten?.();
      term.dispose();
      termRef.current = null;
    };
    // Theme and font size are applied in the following effect so the PTY session survives them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.fontSize = fontSize;
    term.options.theme = terminalTheme(theme);
    term.refresh(0, term.rows - 1);
  }, [fontSize, theme]);

  return <div className="term-host" ref={hostRef} onMouseDown={() => termRef.current?.focus()} />;
});
