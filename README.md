# Unit Agent

Unit Agent is a local-first desktop workspace for macOS, Windows, and Linux. It opens real terminal sessions on the computer it is running on, and it can sign in with [Prysel Auth](https://auth.prysel.com) when a network connection is available. The same terminal core also runs headless as `unit-agent-server`.

Offline mode does not call Prysel. Closing the network, stopping `auth.prysel.com`, or letting a session expire leaves the terminals running.

## Architecture

```
Unit Agent
├── React UI (login, desktop, terminal grid, MCP panel)
└── Tauri / Rust
    ├── TerminalManager  → PTY → detected shell
    ├── AuthService      → @prysel/sso protocol → auth.prysel.com
    ├── ConnectionService
    ├── McpManager       → local MCP servers (stdio)
    └── Storage          → settings file + secret service / encrypted session
```

The TypeScript SDK lives in `vendor/prysel-sso`. The desktop app does not ship that package's client secret in the webview. Rust performs the same `POST /api/sso/sessions` and `POST /api/sso/token` calls.

See [docs/architecture.md](docs/architecture.md), [docs/security.md](docs/security.md), and [docs/prysel-auth.md](docs/prysel-auth.md).

## Requirements

- Node.js 20+
- Rust stable (1.85+; current stable is recommended) and Cargo
- A local shell: bash, zsh, or fish on macOS and Linux; PowerShell or `cmd.exe` on Windows

Desktop builds use the system webview:

| System | Webview and packages |
| --- | --- |
| Linux | WebKitGTK 4.1. See the apt packages below. |
| macOS | WKWebView, included with the system. Xcode command line tools. Minimum 10.15. |
| Windows | WebView2. Visual Studio Build Tools with the C++ workload. The installer can download the WebView2 bootstrapper. |

Debian/Ubuntu packages used to build and run the desktop app:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  patchelf libgtk-3-dev pkg-config libdbus-1-dev
```

`unit-agent-server` is the headless build. It still links the desktop crate, so the same native packages are required on Linux. It does not open a window.

## Configuration

Copy the example and fill in the application registered at Prysel Auth. Do not commit the filled file.

```bash
mkdir -p ~/.config/unit-agent
cp .env.example ~/.config/unit-agent/config.env
```

| Variable | Purpose |
| --- | --- |
| `PRYSEL_AUTH_URL` | Auth UI origin. Default `https://auth.prysel.com`. |
| `PRYSEL_API_URL` | API origin for `/api/sso/*` and `/health`. Defaults to the auth URL. |
| `PRYSEL_APP_ID` | SSO application id. |
| `PRYSEL_CLIENT_SECRET` | SSO client secret. Read only by the native backend. |
| `PRYSEL_REDIRECT_URI` | Loopback callback. Default `http://127.0.0.1:47821/callback`. |
| `PRYSEL_CALLBACK_PORT` | Port Unit Agent binds on `127.0.0.1`. Default `47821`. |
| `PRYSEL_SESSION_TTL_HOURS` | Local session lifetime. Default 168. The SSO exchange does not return a refresh token. |
| `PRYSEL_AGENT_URL` | Optional online agent base URL. Unset means the agent service reports that it is not configured. |

Environment variables override `~/.config/unit-agent/config.env` and a `.env` file in the working directory.

The redirect URI must be allow-listed on the Prysel application. Prysel checks scheme, host, and port. Register:

```text
http://127.0.0.1:47821/callback
```

## Development

```bash
npm install
npm run dev
```

`npm run dev` starts Vite on port 1420 and opens the Unit Agent window.

Other commands:

```bash
npm test          # frontend unit tests
npm run test:rust # PTY, safety, auth protocol, storage tests
npm run lint      # tsc --noEmit
npm run fmt       # cargo fmt
npm run clippy
npm run serve -- --help
```

## Production build

```bash
npm run build
```

Tauri bundles for the operating system you build on (`bundle.targets` is `all`):

- Linux: `deb`, AppImage, and `rpm` under `src-tauri/target/release/bundle/`
- macOS: `.app` and `.dmg`
- Windows: NSIS and MSI

The app runs as the current user. It does not need root or an administrator account.

## Headless server

```bash
npm run serve -- --bind 127.0.0.1:47822
```

The same binary is `unit-agent-server`. `unit-agent serve` starts it from the desktop binary. On a Windows release build the desktop binary has no console, so use `unit-agent-server` there.

```bash
curl http://127.0.0.1:47822/health
curl -X POST http://127.0.0.1:47822/api/v1/terminals \
  -H 'content-type: application/json' -d '{"id":"main"}'
curl -X POST http://127.0.0.1:47822/api/v1/terminals/main/input \
  -H 'content-type: application/json' -d '{"data":"echo Unit Agent\r"}'
```

The default bind is `127.0.0.1:47822` (`UNIT_AGENT_BIND`). Listening on any other address requires `UNIT_AGENT_SERVER_TOKEN`. Send it as `Authorization: Bearer …`. The environment variable is preferred over `--token` because process arguments are visible on the machine. `GET /health` stays open for local probes. Every other route checks the token when one is configured. Destructive commands still stop for confirmation; `POST /api/v1/terminals/{id}/confirm` with `{"approve":true}` is the only way to release them.

## Using the desktop

1. Launch Unit Agent.
2. A saved, unexpired Prysel session opens the workspace. Otherwise the login screen is shown.
3. **Sign in with Prysel Auth** opens the system browser on `auth.prysel.com`. After approval, the browser returns to the loopback callback and the profile appears at the top right.
4. **Work Offline** opens the workspace immediately.
5. The large **+** tile starts a real shell in its own PTY. Each workspace keeps its own process, working directory, and scrollback.
6. **logout** clears the stored session and returns to the login screen. It does not delete workspace layout or kill running terminals; they reattach when you enter the desktop again.

Keyboard shortcuts:

| Shortcut | Action |
| --- | --- |
| Ctrl+Shift+T / Ctrl+Shift+N | New workspace |
| Ctrl+Shift+W | Close the active workspace |
| Ctrl+Shift+L | Toggle dark and light mode |
| Ctrl+Shift+M | MCP and services |
| Ctrl+Shift+C / Ctrl+Shift+V | Copy and paste in the terminal |

Commands such as `rm -rf /`, disk formatting, shutdown, and package removal ask for confirmation before the newline is sent to the shell. The check is intentionally incomplete (shell history and aliases are not visible). Nothing received from Prysel or from an MCP server is executed in a terminal unless you confirm it.

## MCP

The services panel (mcp) connects to a local MCP server over stdio using newline-delimited JSON-RPC, protocol `2024-11-05`. Unit Agent initializes the server, lists tools, and calls a tool only after you press Run. Tool arguments that look destructive need a second confirmation.

Unit Agent is also an MCP server. The desktop listens on `127.0.0.1:47823` and shares its terminals with AI clients. Point Claude or Codex at:

```bash
unit-agent mcp
```

That process attaches to the desktop when it is running. The AI chip in the title bar opens a small activity drawer. Allow and Deny cover commands that need confirmation. Stop AI control rejects new AI actions and leaves the shells running. See [docs/mcp.md](docs/mcp.md) and [docs/ai-control.md](docs/ai-control.md).

MCP server definitions, including environment variables you type, are stored in the settings file with mode `0600` on Unix. They are not uploaded.

## Data on disk

| Path | Contents |
| --- | --- |
| OS secret store (`unit-agent` / `prysel-session`) | Profile session, preferred. Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. |
| `session.bin` in the OS data directory | AES-GCM fallback when the secret store is unavailable |
| `settings.json` in the OS config directory | Theme, window, workspace layout, MCP commands |
| `config.env` beside the settings file | Optional local configuration |

On Linux those directories are `~/.local/share/unit-agent` and `~/.config/unit-agent`. macOS uses `~/Library/Application Support/unit-agent`. Windows uses `%APPDATA%\unit-agent`.

Passwords are never stored. Terminal scrollback is kept in memory for the life of the process (about 256 KB replayed on reattach) and is not written to disk.

## Troubleshooting

- **Prysel Auth is not configured.** Set `PRYSEL_APP_ID` and `PRYSEL_CLIENT_SECRET`, then use Work Offline in the meantime.
- **Redirect URI is not registered.** Add `http://127.0.0.1:47821/callback` to the application's callback URLs.
- **Port 47821 is in use.** Set `PRYSEL_CALLBACK_PORT` and a matching `PRYSEL_REDIRECT_URI`, and register that URI.
- **Shell failed to start.** On macOS and Linux, `$SHELL` must be an executable file, then `/bin/bash`, `/bin/zsh`, or `/bin/sh`. On Windows, PowerShell and `cmd.exe` are the fallbacks (`COMSPEC`).
- **Prysel is unreachable.** The status becomes `degraded` or `offline`. Terminals keep running.
