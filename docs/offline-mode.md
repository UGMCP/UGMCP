# Offline mode

Local tools do not check the network first.

Work Offline opens the desktop without calling `auth.prysel.com`. Terminals, workspaces, files, git, build, test, and the local MCP server keep working when the internet or Prysel is down. `unit_agent_internet_status` and `unit_agent_prysel_status` report that state. They are not a gate.

The MCP listener is a loopback socket. `unit-agent mcp` does not need a network to attach to it. A model that itself needs the internet can still fail; Unit Agent does not.

Online mode adds Prysel sign-in and the optional agent health check. Neither is required to run a shell or an AI tool call.
