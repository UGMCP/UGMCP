# Tools

Control API v1. Every tool returns JSON with `success`. Errors include `code` and `message`. Names that are not in this list return `INVALID_ARGUMENT`. They do not pretend to succeed.

Permission is the effect before the session level is applied. `confirm: yes` means Safe and Confirm ask, and Full still asks when the effect is dangerous. Reads do not ask. Side effects are anything that is not a read.

| Name | Purpose | Permission | Confirm | Side effects |
| --- | --- | --- | --- | --- |
| `unit_agent_app_status` | Name, version, connection, clients, workspace count | read | no | no |
| `unit_agent_status` | Same snapshot, for a reconnecting client | read | no | no |
| `unit_agent_app_version` | App, control API, and MCP versions | read | no | no |
| `unit_agent_app_get_settings` | Theme, permission, MCP flag. No secrets | read | no | no |
| `unit_agent_health` | App, MCP, terminal, Prysel reachability | read | no | no |
| `unit_agent_diagnostics` | Same local health check | read | no | no |
| `unit_agent_mcp_status` | Listener, endpoint, clients | read | no | no |
| `unit_agent_mcp_clients` | Connected AI clients | read | no | no |
| `unit_agent_mcp_start` | Accept tool calls again | confirm | yes | yes |
| `unit_agent_mcp_stop` | Refuse tool calls. Terminals stay up | confirm | yes | yes |
| `unit_agent_mcp_disconnect` | Drop one session. Processes stay up | confirm | yes | yes |
| `unit_agent_ai_sessions` | List sessions | read | no | no |
| `unit_agent_ai_session_status` | Permission and emergency flag | read | no | no |
| `unit_agent_ai_session_permissions` | Current level | read | no | no |
| `unit_agent_ai_session_disconnect` | Disconnect this session | confirm | yes | yes |
| `unit_agent_ai_session_pause` | Pause this session | confirm | yes | yes |
| `unit_agent_ai_session_resume` | Resume this session | confirm | yes | yes |
| `unit_agent_ai_emergency_stop` | Stop new AI actions | confirm | yes | yes |
| `unit_agent_ai_activity` | Recent actions | read | no | no |
| `unit_agent_workspace_list` | Live workspaces | read | no | no |
| `unit_agent_workspace_create` | New PTY. `name`, `directory`, `shell`, `columns`, `rows` | confirm | yes | yes |
| `unit_agent_workspace_close` | Close the PTY. Does not delete files | confirm | yes | yes |
| `unit_agent_workspace_delete` | Remove saved metadata only | confirm | yes | yes |
| `unit_agent_workspace_status` | Pid, cwd, shell, owner, lock | read | no | no |
| `unit_agent_workspace_focus` | Ask the window to show it | confirm | yes | yes |
| `unit_agent_workspace_rename` | Rename saved metadata | confirm | yes | yes |
| `unit_agent_workspace_set_directory` | `cd` in the PTY | confirm | yes | yes |
| `unit_agent_workspace_lock` | Lock the workspace to this session | confirm | yes | yes |
| `unit_agent_workspace_unlock` | Unlock it | confirm | yes | yes |
| `unit_agent_terminal_read` | Output with `offset` and `max_bytes` | read | no | no |
| `unit_agent_terminal_write` | Write bytes to the PTY | confirm | yes | yes |
| `unit_agent_terminal_resize` | `columns`, `rows` | confirm | yes | yes |
| `unit_agent_terminal_status` | Pid, shell, cwd | read | no | no |
| `unit_agent_terminal_interrupt` | Ctrl+C | confirm | yes | yes |
| `unit_agent_terminal_eof` | Ctrl+D | confirm | yes | yes |
| `unit_agent_terminal_restart` | Restart the shell | confirm | yes | yes |
| `unit_agent_execute` | Command in the persistent PTY. `workspace_id`, `command`, `timeout` | safe or confirm, from the command | depends | yes |
| `unit_agent_execute_background` | Start a command and return `running: true` | confirm | yes | yes |
| `unit_agent_execute_cancel` | Ctrl+C | confirm | yes | yes |
| `unit_agent_execute_history` | Recent AI commands, redacted | read | no | no |
| `unit_agent_process_list` | Workspace shell pids only | read | no | no |
| `unit_agent_shell_info` | Shell path | read | no | no |
| `unit_agent_fs_list` | Directory listing inside the workspace | read | no | no |
| `unit_agent_fs_stat` | Type, size, age | read | no | no |
| `unit_agent_fs_exists` | Path check | read | no | no |
| `unit_agent_fs_read` | Text with `offset` and `limit`. Secret files refused | read | no | no |
| `unit_agent_fs_write` | Write inside the workspace root | confirm | yes | yes |
| `unit_agent_fs_append` | Append inside the workspace root | confirm | yes | yes |
| `unit_agent_fs_create_directory` | Create a directory inside the root | confirm | yes | yes |
| `unit_agent_fs_delete` | Delete a file inside the root, not the root | confirm | yes | yes |
| `unit_agent_file_replace` | Replace an exact snippet | confirm | yes | yes |
| `unit_agent_search` | Text search. Skips `.git`, `node_modules`, `target`, `dist`, `build`, `.cache`, `.next`, `.nuxt`, `venv`, `.venv` | read | no | no |
| `unit_agent_git_status` | `git status` in the PTY | read | no | no |
| `unit_agent_git_diff` | `git diff --stat` in the PTY | read | no | no |
| `unit_agent_git_log` | Recent log in the PTY | read | no | no |
| `unit_agent_git_branch_list` | `git branch` in the PTY | read | no | no |
| `unit_agent_git_remote` | Remotes, without credentials | read | no | no |
| `unit_agent_git_commit` | Commit. Message is one line | confirm | yes | yes |
| `unit_agent_git_push` | Push | confirm | yes | yes |
| `unit_agent_project_detect` | Languages and manifests | read | no | no |
| `unit_agent_project_info` | Name, directory, git, tools | read | no | no |
| `unit_agent_package_manager_detect` | npm, cargo, pip, and the other manifests that are present | read | no | no |
| `unit_agent_build` | Detected build in the PTY | safe | no at Safe | yes |
| `unit_agent_test` | Detected test in the PTY | safe | no at Safe | yes |
| `unit_agent_lint` | Detected lint in the PTY | safe | no at Safe | yes |
| `unit_agent_dev_start` | Detected dev server, background | confirm | yes | yes |
| `unit_agent_system_info` | OS, arch, hostname, shell, user | read | no | no |
| `unit_agent_cpu_status` | Load and core count when the OS exposes them | read | no | no |
| `unit_agent_memory_status` | Memory when `/proc/meminfo` exists | read | no | no |
| `unit_agent_disk_status` | Filesystem space for the workspace | read | no | no |
| `unit_agent_network_status` | Hostname and default-route interface | read | no | no |
| `unit_agent_internet_status` | Probe result. Not a prerequisite | read | no | no |
| `unit_agent_prysel_status` | Auth and reachability. No tokens | read | no | no |
| `unit_agent_auth_status` | Stored profile summary | read | no | no |
| `unit_agent_user_profile` | Name, username, email | read | no | no |
| `unit_agent_port_check` | TCP connect to `127.0.0.1` | read | no | no |
| `unit_agent_docker_status` | `docker version` if the binary exists | read | no | no |
| `unit_agent_docker_ps` | `docker ps` if the binary exists | read | no | no |
| `unit_agent_logs_read` | Audit records | read | no | no |
| `unit_agent_audit_list` | Same records | read | no | no |
| `unit_agent_clipboard_read` | Refused | read | no | no |
| `unit_agent_screenshot` | Refused | read | no | no |
| `unit_agent_notification` | Desktop notice | confirm | yes | yes |

`unit_agent_execute` parameters are `workspace_id` (required), `command`, and `timeout` (milliseconds, default 8000, max 120000). It runs in that workspace's existing shell. A command that does not finish returns `COMMAND_TIMEOUT`, `running: true`, and `process_id`.

Package install, archives, service start, and unrestricted database access are not tools in this version. An AI can still run an installed CLI through `unit_agent_execute`, and that command is judged by the same policy. SSH is the real `ssh` program in the PTY. Passwords are not stored.
