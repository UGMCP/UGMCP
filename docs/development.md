# Development

```bash
npm install
npm test
npm run lint
npm run test:rust
npm run clippy
```

`npm run dev` opens the window. The debug build expects Vite on `http://localhost:1420`.

`npm run build:web` writes the frontend to `dist-web/`. Installer commands copy finished packages to `dist/`. See [installers.md](installers.md).

## Control API

Version `1`. MCP protocol `2024-11-05`.

With the desktop running:

```bash
cargo run --manifest-path src-tauri/Cargo.toml -- status
cargo run --manifest-path src-tauri/Cargo.toml -- mcp
```

`unit-agent --headless` is `unit-agent serve`. It serves HTTP on `127.0.0.1:47822` and MCP on `127.0.0.1:47823`, using one `TerminalManager`. Do not install a systemd unit automatically; a user can start the binary themselves.

Rust tests for the control layer use a temporary settings directory. They do not write `~/.config/unit-agent/settings.json`. PTY tests skip when `/dev/pts` is missing.

## Adding a tool

Register it in the `TOOLS` list in `src-tauri/src/control.rs` with an effect (`Read`, `Develop`, `Mutate`, `Dangerous`) and handle it in `dispatch`. The MCP server and the CLI pick it up from that registry. Do not add a second implementation for the UI.
