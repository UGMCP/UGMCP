import { useEffect, useState } from "react";
import { api, asAppError } from "../services/api";
import type { AgentStatus, AppInfo, ConnectionSnapshot, McpServerInfo, Settings } from "../types";

interface Props {
  connection: ConnectionSnapshot;
  settings: Settings;
  onClose: () => void;
  onSettings: (patch: Partial<Settings>) => void;
  onError: (message: string) => void;
}

export function ServicesPanel({ connection, settings, onClose, onSettings, onError }: Props) {
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [agents, setAgents] = useState<{ local: AgentStatus; prysel: AgentStatus } | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [envText, setEnvText] = useState("");
  const [pendingTool, setPendingTool] = useState<{ serverId: string; name: string; args: string; danger: string | null } | null>(null);

  async function refresh() {
    const [list, agent, about] = await Promise.all([api.mcpList(), api.agentStatus(), api.appInfo()]);
    setServers(list);
    setAgents(agent);
    setInfo(about);
  }

  useEffect(() => {
    void refresh().catch((error: unknown) => onError(asAppError(error).message));
  }, [onError]);

  async function connectNew() {
    const env: Record<string, string> = {};
    for (const line of envText.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed || !trimmed.includes("=")) continue;
      const index = trimmed.indexOf("=");
      env[trimmed.slice(0, index)] = trimmed.slice(index + 1);
    }
    try {
      await api.mcpConnect({
        name,
        command,
        args: args.trim() ? args.trim().split(/\s+/) : [],
        env,
      });
      setName("");
      setCommand("");
      setArgs("");
      setEnvText("");
      await refresh();
    } catch (error) {
      onError(asAppError(error).message);
    }
  }

  return (
    <>
      <button className="panel-backdrop" aria-label="Close services" onClick={onClose} />
      <aside className="panel" role="dialog" aria-label="Services">
        <h2>Services</h2>
        <div className="kv">
          <span>Internet</span>
          <strong>{connection.internet.toLowerCase()}</strong>
          <span>Prysel</span>
          <strong>{connection.prysel.toLowerCase()}</strong>
          <span>State</span>
          <strong>{connection.state.toLowerCase()}</strong>
        </div>
        <p>{connection.detail}</p>
        <button className="text-btn" onClick={() => void api.connectionRefresh()}>
          Check connection
        </button>

        <h3>Agent</h3>
        <p>{agents?.local.detail ?? "Checking the local agent."}</p>
        <p>{agents?.prysel.detail ?? "Checking the Prysel agent."}</p>

        <h3>Appearance</h3>
        <label className="field">
          <span>Terminal font size</span>
          <input
            type="number"
            min={10}
            max={22}
            value={settings.fontSize}
            onChange={(event) => onSettings({ fontSize: Number(event.target.value) })}
          />
        </label>
        <label className="field">
          <span>Shell</span>
          <input
            value={settings.shell ?? ""}
            placeholder="Detected from $SHELL"
            onChange={(event) => onSettings({ shell: event.target.value })}
          />
        </label>
        <label className="field">
          <span>Default directory</span>
          <input
            value={settings.defaultCwd ?? ""}
            placeholder="Home directory"
            onChange={(event) => onSettings({ defaultCwd: event.target.value })}
          />
        </label>

        <h3>MCP</h3>
        <p>Connect a local Model Context Protocol server. Tool calls stay on this computer and always ask before they run.</p>
        <label className="field">
          <span>Name</span>
          <input value={name} onChange={(event) => setName(event.target.value)} />
        </label>
        <label className="field">
          <span>Command</span>
          <input value={command} onChange={(event) => setCommand(event.target.value)} placeholder="npx" />
        </label>
        <label className="field">
          <span>Arguments</span>
          <input value={args} onChange={(event) => setArgs(event.target.value)} />
        </label>
        <label className="field">
          <span>Environment</span>
          <textarea rows={3} value={envText} onChange={(event) => setEnvText(event.target.value)} placeholder={"KEY=value"} />
        </label>
        <button className="primary" onClick={() => void connectNew()} disabled={!command.trim()}>
          Connect
        </button>

        {servers.map((server) => (
          <article className="server" key={server.id}>
            <header>
              <strong>{server.name}</strong>
              <span className="meta">{server.status}</span>
            </header>
            <div className="meta">{[server.command, ...server.args].join(" ")}</div>
            {server.message ? <p>{server.message}</p> : null}
            <div className="row-actions">
              {server.running ? (
                <button className="text-btn" onClick={() => void api.mcpDisconnect(server.id).then(refresh)}>
                  Disconnect
                </button>
              ) : (
                <button className="text-btn" onClick={() => void api.mcpStart(server.id).then(refresh).catch((error: unknown) => onError(asAppError(error).message))}>
                  Connect
                </button>
              )}
              <button className="text-btn" onClick={() => void api.mcpForget(server.id).then(refresh)}>
                Remove
              </button>
            </div>
            {server.tools.map((tool) => (
              <div className="tool" key={tool.name}>
                <span title={tool.description}>{tool.name}</span>
                <button
                  className="text-btn"
                  onClick={() => setPendingTool({ serverId: server.id, name: tool.name, args: "{}", danger: null })}
                >
                  Run
                </button>
              </div>
            ))}
          </article>
        ))}

        {pendingTool ? (
          <div className="server">
            <strong>{pendingTool.name}</strong>
            <label className="field">
              <span>Arguments JSON</span>
              <textarea rows={4} value={pendingTool.args} onChange={(event) => setPendingTool({ ...pendingTool, args: event.target.value })} />
            </label>
            {pendingTool.danger ? <p>{pendingTool.danger}</p> : null}
            <div className="row-actions">
              <button className="text-btn" onClick={() => setPendingTool(null)}>
                Cancel
              </button>
              <button
                className="primary"
                onClick={() => {
                  void (async () => {
                    try {
                      const args = JSON.parse(pendingTool.args) as Record<string, unknown>;
                      await api.mcpCallTool({
                        id: pendingTool.serverId,
                        name: pendingTool.name,
                        arguments: args,
                        confirmed: true,
                        acknowledgedDanger: Boolean(pendingTool.danger),
                      });
                      setPendingTool(null);
                    } catch (error) {
                      const parsed = asAppError(error);
                      if (parsed.code === "NEEDS_CONFIRM") {
                        setPendingTool({ ...pendingTool, danger: parsed.message });
                      } else {
                        onError(parsed.message);
                      }
                    }
                  })();
                }}
              >
                {pendingTool.danger ? "Run anyway" : "Run"}
              </button>
            </div>
          </div>
        ) : null}

        <h3>Shortcuts</h3>
        <ul className="shortcuts">
          {(info?.shortcuts ?? []).map((shortcut) => (
            <li key={shortcut.keys}>
              <span>{shortcut.keys}</span>
              <span>{shortcut.action}</span>
            </li>
          ))}
        </ul>
        {info ? (
          <p>
            {info.name} {info.version} on {info.platform}. Auth {info.config.authUrl}. Redirect {info.config.redirectUri}.
          </p>
        ) : null}
      </aside>
    </>
  );
}
