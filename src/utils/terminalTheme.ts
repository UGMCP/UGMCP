import type { ITheme } from "@xterm/xterm";
import type { ThemeName } from "../types";

export function terminalTheme(theme: ThemeName): ITheme {
  if (theme === "light") {
    return {
      background: "#f3f5f8",
      foreground: "#163528",
      cursor: "#1c2433",
      cursorAccent: "#f3f5f8",
      selectionBackground: "#c9d7ee",
      black: "#1c2433",
      green: "#0c7a45",
      brightGreen: "#0a8f4e",
    };
  }
  return {
    background: "#050607",
    foreground: "#3dde67",
    cursor: "#d5dbe6",
    cursorAccent: "#050607",
    selectionBackground: "rgba(90, 122, 190, 0.35)",
    black: "#050607",
    brightBlack: "#3a4254",
    green: "#3dde67",
    brightGreen: "#7dff9a",
  };
}

export const TERMINAL_FONT =
  '"JetBrains Mono", "Fira Code", "Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono", ui-monospace, monospace';
