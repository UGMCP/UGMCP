# AI control

AI control is optional. Work Offline, login, and the workspace grid work with MCP disabled.

## One backend

The window, the CLI, and the MCP server call `ControlHub`. Creating a workspace from an AI adds the same `TerminalInfo` the grid already renders. A command is written to that workspace's PTY, so the user sees `[AI]` and the shell echo. `cd` stays in effect for the next command.

Closing the MCP listener or disconnecting a client does not send SIGHUP to those shells. Closing the Unit Agent window still stops shells the app started, which is the existing desktop shutdown.

## Permission

| Level | Behavior |
| --- | --- |
| Read only | Status, lists, file reads, git inspection. No execution. |
| Safe | Reads plus build, test, and lint. Other changes ask. |
| Confirm | Default. Reads run. Everything else asks. |
| Full control | Runs non-destructive work without a prompt. Destructive work still asks. Not the default. |

The policy looks at the tool and the command. `echo rm -rf /` is a read. `rm -rf /` is dangerous. `git status` is a read. `git push` always asks. The line guard still holds a destructive newline even after the AI prompt, and the control layer releases it only when that same command was the one just approved.

## Confirmation

The drawer shows the client, action, workspace, and command, with Allow and Deny. The AI call waits up to 45 seconds. Deny, or a timeout, returns `CONFIRMATION_REQUIRED` and does not write the command.

Stop AI control sets an emergency flag, drops sessions, unlocks workspaces, and leaves the terminals up. Resume clears the flag.

## What an AI can see

Connected clients appear on the title-bar chip (`AI`, `AI Claude`, or `AI off`). The drawer lists the last few actions. It does not cover the terminal.

## What stays off

Remote control, clipboard capture, screenshots, camera, microphone, and mouse control. Secret files are not returned. Output that looks like a token is replaced with `[REDACTED]`.
