# Security

Unit Agent can run programs as the current user. The local machine stays under the user's control.

## Command execution

Only keystrokes and pastes from the desktop UI, or an authenticated request to the headless server, are written to a PTY. Prysel responses and MCP tool results are never piped into a shell.

Before a newline is forwarded, the current input line is checked for a short list of destructive patterns:

- `rm -rf` aimed at `/`, `/*`, `~`, `$HOME`, `.`, `*`, or a top-level system directory
- `mkfs`, `wipefs`, `dd` onto a device, `shred` of a device
- `shutdown`, `reboot`, `poweroff`, `halt`
- package removal (`apt remove`, `dnf remove`, `pacman -R`, `snap remove`, and the usual variants)
- `chmod -R 777 /` and a fork bomb

The newline is held until the user chooses Run or Cancel. Cancel sends Ctrl-U. This is a barrier, not a complete policy. Shell history, aliases, and commands typed inside `vim` or `ssh` are not reconstructed.

`sudo` is passed through to the real sudo prompt. Unit Agent does not store a sudo password and does not run as root.

## Remote instructions

MCP `tools/call` requires `confirmed: true`. If the JSON arguments contain a destructive command string, the backend also requires `acknowledgedDanger`. The services panel shows that second step.

There is no command that accepts a payload from `auth.prysel.com` and executes it.

## Authentication

The client secret is read from the environment or `~/.config/unit-agent/config.env` inside the Rust process. It is not compiled into the frontend, not placed in a `VITE_` variable, and not written to logs.

The browser flow uses the system browser and a loopback callback bound to `127.0.0.1` only. The `state` parameter is a random value checked before the code is exchanged. Callback errors are not reflected into the HTML page.

The token exchange returns a user profile, not an access token or refresh token. Unit Agent stores that profile:

1. In the OS secret store when it answers within 1.5 seconds (Secret Service, macOS Keychain, or Windows Credential Manager).
2. Otherwise in `session.bin`, encrypted with AES-256-GCM. The key is derived from a machine id and the user id. Linux uses `/etc/machine-id` (or `/var/lib/dbus/machine-id`) and the uid. macOS uses the platform UUID and the uid. Windows uses `MachineGuid` and `USERNAME`. On Unix the file mode is `0600`. On Windows the file stays in the per-user profile.

Logout deletes both copies. Passwords are never stored.

A restored profile is a local session. It is not described as an online authentication until the user completes the browser flow again in this process.

## Headless server

`unit-agent-server` binds to loopback unless `UNIT_AGENT_BIND` or `--bind` says otherwise. A non-loopback address is refused until `UNIT_AGENT_SERVER_TOKEN` is set. When a token is set, every route except `GET /health` requires `Authorization: Bearer`. The comparison covers the whole token. The server does not send a CORS header. Terminal input still passes through the destructive-command guard, and a held command runs only after `POST .../confirm` with `approve: true`.

## Network

Requests time out (about 4–20 seconds depending on the call). Connectivity is checked every 30 seconds, with one Prysel HTTP request per pass. Losing the network does not log the user out and does not close terminals.

## Privacy

Terminal output, command history, environment variables, and files stay on the machine. They are not synchronized to Prysel. MCP environment values the user enters are local settings and are not logged.

Logs omit authorization codes, client secrets, and session JSON. API error text that is empty, HTML, or mentions a secret is replaced with a generic message.

## Logging and debug mode

Debug mode can show connection state, workspace ids, process ids, the application id, and whether a secret is configured. It does not show the secret or terminal output.

## Application permissions

The package runs as a normal desktop user. Installation of a `.deb` may require privileges; running Unit Agent does not.

## AI control

Local MCP authorization is separate from Prysel login. Prysel authenticates the person. The MCP permission level decides what an AI may do on this computer. The default is `confirm`. `full-control` is never the default, and destructive commands still ask even then.

The listener is `127.0.0.1:47823`. `0.0.0.0` is rejected in code. There is no remote-control switch that opens a public socket in this version.

AI writes go through the same PTY and the same line guard as the keyboard. A denied confirmation is not written. Secret paths (`.env`, private keys, `~/.ssh/id_rsa`, `id_ed25519`, `id_ecdsa`, `.pem` private keys) are refused. Paths that escape the workspace root are refused unless the level is full control, and secret paths stay refused at every level. Returned text replaces likely tokens with `[REDACTED]`.

Clipboard and screenshot tools stay denied. This build does not capture them even if the settings flag is turned on. There is no camera, microphone, or mouse control.

Text inside a repository or a terminal is not treated as an instruction. Only a tool call that passes the permission check runs.

Audit rows keep the client, tool, workspace, and result. They do not keep passwords or tokens. Disconnecting an AI session unlocks its workspace and leaves the shell running. Stop AI control rejects new actions and does not close terminals.
