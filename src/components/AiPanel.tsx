import type { AiActivityEntry, AiClient, AiConfirmRequest, Settings } from "../types";

interface Props {
  settings: Settings;
  clients: AiClient[];
  activity: AiActivityEntry[];
  stopped: boolean;
  confirm: AiConfirmRequest | null;
  onClose: () => void;
  onSettings: (patch: Partial<Settings>) => void;
  onConfirm: (allow: boolean) => void;
  onStop: () => void;
  onResume: () => void;
}

function clock(at: number): string {
  return new Date(at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function AiPanel({
  settings,
  clients,
  activity,
  stopped,
  confirm,
  onClose,
  onSettings,
  onConfirm,
  onStop,
  onResume,
}: Props) {
  const names = clients.map((client) => client.client).filter(Boolean);
  return (
    <>
      <button className="panel-backdrop" aria-label="Close AI control" onClick={onClose} />
      <aside className="panel ai-panel" role="dialog" aria-label="AI control">
        <h2>AI control</h2>
        <p className="ai-lead">
          {stopped ? "AI control disabled" : names.length ? names.join(", ") : "No AI connected"}
        </p>
        {confirm ? (
          <div className="confirm-card" role="alertdialog" aria-label="Allow AI action">
            <p><span>AI client</span> {confirm.client || "MCP"}</p>
            <p><span>Action</span> {confirm.action}</p>
            <p><span>Workspace</span> {confirm.workspace || "current"}</p>
            <p><span>Command</span> <code>{confirm.command}</code></p>
            <div className="row-actions">
              <button className="text-btn" onClick={() => onConfirm(true)}>Allow</button>
              <button className="text-btn danger-btn" onClick={() => onConfirm(false)}>Deny</button>
            </div>
          </div>
        ) : null}
        <label className="field">
          <span>MCP server</span>
          <select
            value={settings.aiMcp ? "on" : "off"}
            onChange={(event) => onSettings({ aiMcp: event.target.value === "on" })}
          >
            <option value="on">Enabled</option>
            <option value="off">Disabled</option>
          </select>
        </label>
        <label className="field">
          <span>Permission</span>
          <select
            value={settings.aiPermission || "confirm"}
            onChange={(event) => onSettings({ aiPermission: event.target.value })}
          >
            <option value="read-only">Read only</option>
            <option value="safe">Safe</option>
            <option value="confirm">Confirm</option>
            <option value="full-control">Full control</option>
          </select>
        </label>
        <div className="kv">
          <span>Remote</span><strong>Disabled</strong>
          <span>Audit</span><strong>Enabled</strong>
          <span>Connected</span><strong>{clients.length}</strong>
        </div>
        <h3>AI activity</h3>
        {activity.length === 0 ? <p>No AI actions yet.</p> : (
          <ul className="ai-activity">
            {activity.slice(0, 8).map((entry, index) => (
              <li key={`${entry.at}-${index}`}>
                <span>{clock(entry.at)}</span>
                <span>{entry.status === "success" ? "✓" : entry.status === "error" || entry.status === "denied" ? "×" : "●"}</span>
                <span>{entry.client}</span>
                <code>{entry.action}</code>
              </li>
            ))}
          </ul>
        )}
        <div className="row-actions">
          {stopped ? (
            <button className="text-btn" onClick={onResume}>Resume AI</button>
          ) : (
            <button className="text-btn danger-btn" onClick={onStop}>Stop AI control</button>
          )}
        </div>
      </aside>
    </>
  );
}
