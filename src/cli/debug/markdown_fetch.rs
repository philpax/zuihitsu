//! The `markdown-fetch` command: drive the real `web.markdown` pipeline — the reqwest transport,
//! the readability extraction, and the Markdown rendering — against one URL, and print the Markdown
//! the agent would receive. The one debug command that reaches the network, so what it shows is
//! exactly what a live fetch produces, stored settings included.

use std::{sync::Arc, time::Duration};

use zuihitsu::{
    HttpFetcher, HttpFetcherConfig, Settings, SqliteStore, WebClient, WebSettings,
    config::EnvConfig,
};

use crate::cli::error::CliError;

/// Fetch `url` through the real pipeline and print the resulting Markdown to stdout — the command's
/// machine-readable product, so it stays pipeable; diagnostics go to stderr via `tracing`. The web
/// settings come from the config-selected event log when one exists (the values a running agent
/// reads), else the defaults. `allow_private` opens the SSRF guard for this one invocation, so a
/// local dev page can be inspected without touching the stored settings.
pub(crate) fn markdown_fetch(
    config: &EnvConfig,
    url: &str,
    allow_private: bool,
) -> Result<(), CliError> {
    let web = web_settings(config);
    let fetcher = HttpFetcher::new(HttpFetcherConfig {
        timeout: Duration::from_secs(web.fetch_timeout_seconds.max(1) as u64),
        max_response_bytes: web.max_response_bytes.max(0) as u64,
        user_agent: web.user_agent.clone(),
        allow_private_addresses: allow_private || web.allow_private_addresses,
    })
    .map_err(|source| CliError::MarkdownFetch(source.to_string()))?;
    let client = WebClient::new(Arc::new(fetcher), web.max_markdown_chars.max(0) as usize);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            CliError::MarkdownFetch(format!("could not start the async runtime: {source}"))
        })?;
    let markdown = runtime
        .block_on(client.markdown(url))
        .map_err(|source| CliError::MarkdownFetch(source.to_string()))?;
    println!("{markdown}");
    Ok(())
}

/// The web settings the fetch runs under: read from the config-selected event log when it exists —
/// opened read-only, so the command is safe while the agent runs — else the defaults, so the
/// pipeline can be debugged before any agent exists. A log that cannot be read falls back to the
/// defaults too, with a warning, since the fetch itself is still worth running.
fn web_settings(config: &EnvConfig) -> WebSettings {
    let path = config.storage.event_log();
    if !path.exists() {
        tracing::info!(
            path = %path.display(),
            "no event log; fetching with the default web settings"
        );
        return WebSettings::default();
    }
    let stored = SqliteStore::open_read_only(&path)
        .map_err(|source| source.to_string())
        .and_then(|store| Settings::from_store(&store).map_err(|source| source.to_string()));
    match stored {
        Ok(settings) => settings.web,
        Err(error) => {
            tracing::warn!(
                %error,
                path = %path.display(),
                "could not read the stored settings; fetching with the default web settings"
            );
            WebSettings::default()
        }
    }
}
