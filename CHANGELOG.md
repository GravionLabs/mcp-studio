# Changelog

All notable changes to GravionLabs/mcp-studio are documented here.
Versioning follows [Semantic Versioning](https://semver.org/).

---

## [0.1.3] — 2026-10-01

### Documentation
- Merge every pull request into the epic's release branch (#187)


### Features
- Token metering and tracing (epic #96) (#193)
- Show context cost per server and cost per call and session (#100) (#188)
- Record sessions and tool calls as spans (#102) (#189)
- Show sessions as a waterfall of spans (#103) (#190)
- Export spans to an OpenTelemetry collector (#104) (#191)
- Exact token counts through Anthropic's token counting endpoint (#99) (#192)


## [0.1.2] — 2026-10-01

### Features
- Estimate tokens for tool definitions, arguments, and results (#98) (#185)


## [0.1.1] — 2026-10-01

### Features
- Check for and install updates with tauri-plugin-updater (#140) (#184)


## [0.1.0] — 2026-10-01

### Bug Fixes
- Keep playground input stable when saved requests or history reload (#77)
- Stop the explorer effect from retriggering itself (#169) (#170)
- Find stdio server programs like a shell does (#173) (#174)
- Let the server detail view use the full pane width (#176)
- Start release versioning at 0.1.0 (#181)


### Documentation
- Add product spec, architecture, data model, and ADR 0001
- Choose a license and add contribution guidelines (#135) (#165)
- Bring the specs up to date with the MVP (#172)
- Add a step-by-step guide to recording a real client (#178)
- Add a security policy (#180)


### Features
- Scaffold Tauri 2 + Angular 22 app with Cargo workspace (#3) (#141)
- Generate TypeScript bindings and typed IPC service (#12) (#143)
- Add SQLite storage with migrations (#16) (#144)
- Add SQLite storage with migrations (#16) (#145)
- Spike rmcp connectivity and message recording (#32) (#146)
- Add status bar and toast notifications (#27) (#147)
- Add three-column app shell with themes (#23) (#148)
- Manage server definitions (#37) (#149)
- Store secrets in the OS keyring (#19) (#150)
- Environments and {{variable}} placeholders (#43) (#151)
- Persist and query recorded MCP messages (#55) (#152)
- Connect to and disconnect from MCP servers (#47) (#153)
- Explore tools, resources, and prompts of a server (#61) (#154)
- Call tools with a generated form (#65) (#155)
- Render tool results by content type (#69) (#156)
- Read resources and get prompts (#71) (#157)
- Save requests in collections (#74) (#158)
- Request history with search and rerun (#77) (#159)
- Request history with search and rerun (#77)
- Live message timeline in the inspector (#81) (#160)
- Compare two recorded messages (#85) (#161)
- Record real client sessions through a stdio proxy (#88) (#162)
- Record real client sessions through a stdio proxy (#88)
- Record remote servers through a local HTTP proxy (#92) (#163)
- Copy client configuration for the proxy (#94) (#164)
- Sign in to Streamable HTTP servers with OAuth 2.1 (#51) (#167)
- Import servers from Claude Desktop and Claude Code configs (#40) (#168)
- Flow graph model and validation (#107) (#175)
- Pick the client for proxy setup snippets (#176) (#177)
- Pick the client for proxy setup snippets (#176)



