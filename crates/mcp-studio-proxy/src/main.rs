//! `mcp-studio-proxy --server <id-or-name> [--discovery <file>]`
//!
//! Point an MCP client (Claude Code, Claude Desktop, an IDE) at this program instead of the real
//! server. It connects to the running MCP Studio app, which starts the real server, records every
//! message, and forwards them unchanged. This program only pipes bytes between the client's stdio
//! and the app.

use std::path::PathBuf;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

/// Written by the app to `<app data dir>/proxy.json`.
#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}

#[derive(Serialize)]
struct Hello<'a> {
    token: &'a str,
    server: &'a str,
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

struct Args {
    server: String,
    discovery: PathBuf,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut server = None;
    let mut discovery = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = args.next(),
            "--discovery" => discovery = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                eprintln!("usage: mcp-studio-proxy --server <id-or-name> [--discovery <file>]");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}"),
        }
    }
    let server = server.context("--server <id-or-name> is required")?;
    let discovery = discovery
        .or_else(|| std::env::var_os("MCP_STUDIO_DISCOVERY").map(PathBuf::from))
        .or_else(|| {
            dirs::data_dir().map(|d| d.join("dev.gravionlabs.mcp-studio").join("proxy.json"))
        })
        .context("cannot locate the MCP Studio data directory")?;
    Ok(Args { server, discovery })
}

async fn run() -> anyhow::Result<()> {
    let args = parse_args()?;
    let text = std::fs::read_to_string(&args.discovery).with_context(|| {
        format!(
            "MCP Studio does not seem to be running (could not read {}). Start the app and try again.",
            args.discovery.display()
        )
    })?;
    let discovery: Discovery = serde_json::from_str(&text).context("proxy.json is not valid")?;

    let stream = TcpStream::connect(("127.0.0.1", discovery.port))
        .await
        .context("could not reach MCP Studio; is the app running?")?;
    let (read_half, mut write_half) = stream.into_split();

    let mut hello = serde_json::to_vec(&Hello {
        token: &discovery.token,
        server: &args.server,
    })?;
    hello.push(b'\n');
    write_half.write_all(&hello).await?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let reply: Reply =
        serde_json::from_str(line.trim()).context("unexpected answer from MCP Studio")?;
    if !reply.ok {
        bail!(
            "{}",
            reply
                .error
                .unwrap_or_else(|| "MCP Studio refused the connection".into())
        );
    }

    // Client -> app, then close our write side so the app can end the server's stdin.
    let upstream = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut write_half).await;
        let _ = write_half.shutdown().await;
    });
    // App -> client. Ends when the app closes the connection.
    let mut stdout = tokio::io::stdout();
    tokio::io::copy(&mut reader, &mut stdout).await?;
    stdout.flush().await?;
    upstream.abort();
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("mcp-studio-proxy: {error:#}");
        std::process::exit(1);
    }
}
