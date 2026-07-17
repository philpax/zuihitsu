//! The `mcp` command: list the tools each configured MCP server exposes by connecting to the servers
//! directly, so an operator can see a catalogue before narrowing it with `allow`/`deny`.

use zuihitsu::{McpHost, McpTool, config::EnvConfig};

use crate::cli::error::CliError;

/// List the tools each configured MCP server exposes. Connects to the servers directly (no running
/// agent needed), snapshots each catalogue, and prints it as a readable listing — a server that cannot
/// be brought up reports its error and the rest still run, so one missing binary does not hide the
/// others. The operator reads this to choose an `allow`/`deny` projection.
pub(crate) fn mcp(config: &EnvConfig) -> Result<(), CliError> {
    if config.mcp.is_empty() {
        tracing::info!("no MCP servers configured; add an [mcp.<name>] block to the config");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| CliError::Mcp(format!("could not start the async runtime: {source}")))?;
    runtime.block_on(async {
        let host = zuihitsu::RmcpHost;
        for (name, server) in &config.mcp {
            match host.spawn(name, server).await {
                Ok(instance) => {
                    print_catalogue(name, instance.tools());
                    instance.shutdown().await;
                }
                Err(error) => println!("{name}\n  could not spawn: {error}\n"),
            }
        }
    });
    Ok(())
}

/// Print one server's catalogue: a header with its tool count, then each tool's name (aligned) and
/// description. Plain text, so it stays legible piped or redirected.
fn print_catalogue(name: &str, tools: &[McpTool]) {
    let plural = if tools.len() == 1 { "" } else { "s" };
    println!("{name} · {} tool{plural}", tools.len());
    // Align names into a column, but cap the width so one long name does not push every description out.
    let width = tools
        .iter()
        .map(|tool| tool.name.len())
        .max()
        .unwrap_or(0)
        .min(24);
    for tool in tools {
        println!("  {:<width$}  {}", tool.name, tool.description);
    }
    println!();
}
