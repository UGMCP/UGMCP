# Architecture

Unit Agent is a Tauri 2 desktop application. The React UI renders the workspace. Rust owns processes, credentials, and network calls that carry the Prysel client secret.

```
                         Unit Agent
                             |
              +--------------+--------------+
              |                             |
              v                             v
       Local environment              Prysel cloud
              |                             |
              v                             v
       Local shell / PTY              auth.prysel.com
       Local files                    SSO /api/sso/*
       MCP stdio servers              optional agent URL
```

## UI

| Path | Role |
| --- | --- |
| `src/components/LoginScreen.tsx` | Sign in and Work Offline |
| `src/components/Desktop.tsx` | Responsive workspace grid |
| `src/components/WorkspacePanel.tsx` | One terminal, close and confirm chrome |
| `src/components/TerminalView.tsx` | xterm.js bound to a PTY |
| `src/components/ServicesPanel.tsx` | Connection, appearance, MCP |
| `src/stores/AppStore.tsx` | Session, connection, and workspace state |
| `src/services/api.ts` | Typed Tauri commands |

The grid is one column below 900px, two columns from 900px (including 1366×768), and three columns from 1500px (including 1920×1080, 1440p, and 4K). Panels are CSS grid cells and do not overlap.

Dark is the default. Light mode is stored in settings and mirrored to `localStorage` so the first paint can match.

## Native backend

```
src-tauri/src
├── lib.rs            window, command registration, connectivity loop
├── control.rs        shared control API used by the UI, CLI, and MCP
├── mcp_host.rs       local MCP server on 127.0.0.1:47823
├── server.rs         headless HTTP server (unit-agent-server)
├── commands.rs       Tauri IPC
├── terminal.rs       PTY sessions
├── auth.rs           Prysel SSO desktop adapter
├── connection.rs     internet vs Prysel reachability
├── mcp.rs            stdio MCP client
├── agent.rs          LocalAgentService and PryselAgentService
├── storage.rs        settings and encrypted session
├── safety.rs         destructive-command guard
└── system.rs         shell detection and orphan cleanup
```

### Terminal path

```
xterm.js onData / resize
    → Tauri command
    → TerminalManager
    → portable-pty
    → detected shell
        macOS and Linux: $SHELL, then bash, zsh, sh
        Windows: PowerShell, then cmd.exe
    → this computer
```

Output is read on a thread and emitted as `terminal-event`. The webview writes those bytes into xterm and does not keep a second copy in React state. Each workspace is a separate child process. Closing one session does not signal the others.

Bash, zsh, and fish are started with a small rc that sources the user's own rc and reports the working directory with OSC 7. PowerShell sets the same OSC 7 sequence from its prompt. `cmd.exe` is started with `/Q /K`. The frontend and the backend both record that path.

On Linux, shells left behind by a crashed Unit Agent (parent pid 1 and `UNIT_AGENT=1` in the environment) receive SIGHUP. macOS and Windows do not scan another process's environment, so only shells Unit Agent still tracks are stopped. A normal exit hangs up those shells on Unix and kills them on every platform.

### Server

`unit-agent-server` (and `unit-agent serve`) runs `TerminalManager` without a window. It listens on `127.0.0.1:47822` and exposes `/health` plus `/api/v1/terminals`. Writes go through the same line guard as the desktop. Binding a non-loopback address requires `UNIT_AGENT_SERVER_TOKEN`.

### Online path

```
ConnectionService every 30s
    → TCP probe to public resolvers
    → GET {PRYSEL_API_URL}/health or /api/config
    → ONLINE | OFFLINE | DEGRADED | CHECKING | ERROR
```

`DEGRADED` means the internet is reachable and Prysel is not. The UI does not return to the login screen when this happens.

### Authentication path

```
AuthService
    → bind 127.0.0.1:47821
    → POST /api/sso/sessions   (same body as @prysel/sso createAuthorizeUrl)
    → system browser → authorize_url
    → loopback GET /callback?code&state
    → POST /api/sso/token      (same body as exchangeCode)
    → profile stored in Secret Service or an encrypted file
```

`PryselAgentService` only calls `PRYSEL_AGENT_URL/health` when that variable is set. Otherwise it reports `Online agent service is not configured.`

`LocalAgentService` reports the detected shell. It does not pretend a local model is installed.

### MCP client

`McpManager` spawns a process with stdin/stdout pipes, sends `initialize`, `notifications/initialized`, and `tools/list`, then `tools/call` after the UI sets `confirmed`. Messages are one JSON object per line. Server environment variables stay in the local settings file. That client is unchanged: Unit Agent can still call tools on other local MCP servers.

### MCP server and control API

The desktop process also exposes Unit Agent itself as an MCP server. `ControlHub` owns no second terminal pool. It holds the same `Arc<TerminalManager>` as the window. `unit-agent mcp` attaches to `127.0.0.1:47823` when the desktop is up, so Claude, Codex, and other MCP clients type into those PTYs. If nothing is listening, stdio mode starts a separate headless hub and says so; those PTYs are not the window's.

Remote bind addresses are rejected. See [mcp.md](mcp.md), [ai-control.md](ai-control.md), and [tools.md](tools.md).

## Persistence

Non-secret settings (theme, font size, window geometry, workspace ids and directories, MCP command lines) are JSON in the OS config directory.

The session record is the profile returned by the token exchange plus an expiry. There is no refresh token in the SSO response, so expiry means the user signs in again. A restored session is labeled `local` until a sign-in completes in the current process. A fresh sign-in is labeled `online`.

## Shutdown

On window exit the app cancels an in-progress login, stops the local MCP listener, stops MCP client children, and hangs up terminal shells it started. Workspace metadata stays on disk. An AI client disconnecting does not hang up those shells.
