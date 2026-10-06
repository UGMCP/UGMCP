# MCP

Unit Agent speaks MCP in two directions.

## Client

The services panel starts other MCP servers over stdio. Protocol `2024-11-05`, one JSON object per line. A tool runs only after the user presses Run. Destructive arguments need a second confirmation. This client is not the AI control path.

## Server

The desktop and `unit-agent --headless` listen on `127.0.0.1:47823`. The bind address must be loopback. `0.0.0.0` is refused before the socket opens. If the port is already taken, the app logs `MCP unavailable` and the terminals keep working.

```
Claude / Codex / other MCP client
        |
        |  stdio: unit-agent mcp
        v
127.0.0.1:47823
        |
        v
ControlHub
        |
        v
TerminalManager  (the same PTYs as the window)
```

`unit-agent mcp` connects to that port and copies stdin and stdout. If the desktop is not running, it starts a separate headless hub and prints that those PTYs are not the window's.

## Handshake

`initialize` opens an AI session from `clientInfo.name`. `tools/list` returns the registry. `tools/call` runs one tool and returns:

```json
{ "content": [{ "type": "text", "text": "{...}" }], "isError": false }
```

The text is JSON with `success`. Failures use codes such as `PERMISSION_DENIED`, `CONFIRMATION_REQUIRED`, `WORKSPACE_BUSY`, `AI_DISABLED`, `MCP_DISABLED`, `PATH_NOT_ALLOWED`, and `COMMAND_TIMEOUT`. A broken JSON line returns a parse error and does not stop the process.

Notifications (no `id`) are ignored.

## Resources

| URI | Contents |
| --- | --- |
| `unit-agent://status` | App, connection, clients |
| `unit-agent://workspaces` | Live workspaces |
| `unit-agent://ai/sessions` | Connected clients |
| `unit-agent://system` | OS and shell, no secrets |

## CLI

```bash
unit-agent status
unit-agent mcp status
unit-agent workspace list
unit-agent diagnostics
```

These call the running control API. They do not start a second terminal if the desktop is already up.
