/** Quotes a command-line argument for display in a POSIX-like shell. */
export function shellQuote(value: string): string {
  return /^[\w@%+=:,./-]+$/.test(value) ? value : `'${value.replace(/'/g, `'\\''`)}'`;
}

/** The command a client should run to reach a server through the proxy. */
export function proxyCommand(binary: string, serverName: string): string {
  return [binary, "--server", serverName].map(shellQuote).join(" ");
}
