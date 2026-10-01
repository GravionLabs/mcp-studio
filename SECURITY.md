# Security Policy

MCP Studio connects to servers you configure, spawns local programs, and stores credentials, so we
take security reports seriously.

## Supported versions

MCP Studio is pre-1.0. Only the latest release and the `main` branch receive security fixes.

## Reporting a vulnerability

**Please do not open a public issue or pull request for a security problem.**

Report it privately through GitHub:
[Report a vulnerability](https://github.com/GravionLabs/mcp-studio/security/advisories/new)
(repository **Security** tab → **Report a vulnerability**).

Please include:

- what the problem is and which component it affects (app, proxy, recording, storage, OAuth, ...)
- the version or commit, and your operating system
- steps to reproduce, or a proof of concept
- the impact you expect, such as a leaked credential or code running without consent

We aim to acknowledge a report within 7 days and to share an assessment and a plan within 30 days.
We will keep you informed, credit you in the advisory if you wish, and ask you to keep the details
private until a fix is released.

## Scope

In scope:

- credentials leaking out of the OS keyring into the database, logs, recordings, exports, or the UI
- tool calls, process launches, or network requests that happen without the user's consent
- the webview gaining access it should not have (spawning processes, reading credentials)
- flaws in the OAuth flow, the stdio proxy, or the HTTP proxy
- unsafe handling of imported or shared files (collections, flows, client configs)

Out of scope:

- vulnerabilities in third-party MCP servers or LLM providers that you connect to
- issues that need an attacker who already controls your machine or your user account
- findings that only apply to an unmodified development build with debugging features enabled
- denial of service through very large inputs from a server you chose to trust

## Handling secrets

MCP Studio is local first: it sends no telemetry and only contacts the servers and providers you
configure. Secrets belong in the OS keyring; the database and logs should only ever contain
`keyring:` references or masked values. If you find a place where this does not hold, that is a
vulnerability, so please report it.
