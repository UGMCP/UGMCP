import { useEffect, useState } from "react";
import type { TerminalInfo, ThemeName } from "../types";
import { gridColumns } from "../utils/startup";
import { PlusTile } from "./PlusTile";
import { WorkspacePanel } from "./WorkspacePanel";

interface Props {
  workspaces: TerminalInfo[];
  activeId: string | null;
  fontSize: number;
  theme: ThemeName;
  generations: Record<string, number>;
  onCreate: () => void;
  onFocus: (id: string) => void;
  onCwd: (id: string, cwd: string) => void;
  onClosed: (id: string) => void;
  onRestarted: (info: TerminalInfo) => void;
  onError: (message: string) => void;
}

export function Desktop({
  workspaces,
  activeId,
  fontSize,
  theme,
  generations,
  onCreate,
  onFocus,
  onCwd,
  onClosed,
  onRestarted,
  onError,
}: Props) {
  const [width, setWidth] = useState(window.innerWidth);
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  const columns = gridColumns(width);
  const solo = workspaces.length === 0;

  return (
    <div className={solo ? "desktop solo" : "desktop"}>
      <div className="grid" style={solo ? undefined : { gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
        {workspaces.map((workspace) => (
          <WorkspacePanel
            key={workspace.id}
            info={workspace}
            active={workspace.id === activeId}
            fontSize={fontSize}
            theme={theme}
            generation={generations[workspace.id] ?? 0}
            onFocus={() => onFocus(workspace.id)}
            onCwd={(cwd) => onCwd(workspace.id, cwd)}
            onClosed={() => onClosed(workspace.id)}
            onRestarted={onRestarted}
            onError={onError}
          />
        ))}
        <PlusTile onClick={onCreate} />
      </div>
    </div>
  );
}
