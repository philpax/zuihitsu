//! The `web` global: `web.markdown(url)`, which fetches a page and returns its main content as
//! Markdown.

use super::*;
use crate::web::WebError;

/// The `web` global: `web.markdown(url)`, which fetches `url`, extracts the page's main content
/// (dropping nav, footers, and other chrome), and returns it as Markdown. Async — the fetch suspends
/// the block until the page answers or the per-fetch timeout fires — but it touches no memory, so it
/// takes no lock, and it deliberately does not latch the block's "made an external call" flag the way
/// an MCP call does: a GET is idempotent, so a block that only fetched can still abort-and-retry on a
/// lock-wait timeout (spec §Concurrency → timeout-and-retry) rather than surfacing the timeout at once.
pub(in crate::agent::lua) fn web_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let web = lua.create_table()?;
    web.set(
        "markdown",
        lua.create_async_function({
            let api = api.clone();
            move |_, url: Value| {
                let api = api.clone();
                async move {
                    let url = url_arg(url)?;
                    let client = api.web.as_ref().ok_or_else(|| {
                        mlua::Error::RuntimeError(
                            "web: fetching is not configured on this instance".to_owned(),
                        )
                    })?;
                    client.markdown(&url).await.map_err(web_to_lua)
                }
            }
        })?,
    )?;
    Ok(web)
}

/// Extract the URL argument. `web.markdown` takes a bare URL string; a non-string argument (a table,
/// most likely, from mistaking it for an MCP-style call) gets a pointed error rather than mlua's
/// opaque conversion failure.
fn url_arg(value: Value) -> mlua::Result<String> {
    match value {
        Value::String(url) => Ok(url.to_string_lossy()),
        other => Err(mlua::Error::RuntimeError(format!(
            "web.markdown takes a URL string, e.g. web.markdown(\"https://example.com\") — got a {}",
            other.type_name()
        ))),
    }
}

/// Render a [`WebError`] as the catchable Lua error the agent sees — its `Display` is the agent-facing
/// wording (teachable prose leading with a `web:` context prefix).
fn web_to_lua(error: WebError) -> mlua::Error {
    mlua::Error::RuntimeError(error.to_string())
}
