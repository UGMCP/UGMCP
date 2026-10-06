# Security

Unit Agent can run programs as the current user. The local machine stays under the user's control.

## Command execution

Only keystrokes and pastes from the desktop UI are written to a PTY. Prysel responses and MCP tool results are never piped into a shell.

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

1. In the desktop Secret Service when it answers within 1.5 seconds.
2. Otherwise in `session.bin`, encrypted with AES-256-GCM. The key is derived from `/etc/machine-id` and the user id. The file mode is `0600`.

Logout deletes both copies. Passwords are never stored.

A restored profile is a local session. It is not described as an online authentication until the user completes the browser flow again in this process.

## Network

Requests time out (about 4–20 seconds depending on the call). Connectivity is checked every 30 seconds, with one Prysel HTTP request per pass. Losing the network does not log the user out and does not close terminals.

## Privacy

Terminal output, command history, environment variables, and files stay on the machine. They are not synchronized to Prysel. MCP environment values the user enters are local settings and are not logged.

Logs omit authorization codes, client secrets, and session JSON. API error text that is empty, HTML, or mentions a secret is replaced with a generic message.

## Logging and debug mode

Debug mode can show connection state, workspace ids, process ids, the application id, and whether a secret is configured. It does not show the secret or terminal output.

## Application permissions

The package runs as a normal desktop user. Installation of a `.deb` may require privileges; running Unit Agent does not.
