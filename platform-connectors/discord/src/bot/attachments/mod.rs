//! Relaying the files a Discord message carried: what to fetch, what to upload, and what to say
//! about whatever did not come through.
//!
//! The two halves are kept apart deliberately, and now by file. Here, [`plan`] decides — from the
//! names, media types, and sizes Discord reports, and the connector's caps — which files are worth
//! fetching and which are excluded before a byte moves; [`relay`](relay::relay) then drives the
//! download and the blob upload for the ones that survived. Every exclusion and every failure
//! produces a note appended to the message text, because the agent knowing that `build.log` was
//! shared and did not arrive is the point: a silently dropped file reads to the agent as a message
//! that carried nothing.

mod relay;
#[cfg(test)]
mod tests;

use serenity::all::Attachment as DiscordAttachment;
use std::fmt;

use zuihitsu_platform_connector_api::{MessageAttachment, PlatformClient};

use crate::{bot::attachments::relay::relay, config::AttachmentConfig};

/// What one message's files came to: the records for those that arrived, and a plain-language note
/// for each that did not.
#[derive(Default)]
pub(super) struct Collected {
    pub attachments: Vec<MessageAttachment>,
    pub notes: Vec<String>,
}

/// Fetch and upload the files a message carried, returning their records alongside a note for each
/// file that did not come through. A failure never propagates: the message is delivered either way,
/// carrying the file or carrying the reason it is absent.
pub(super) async fn collect(
    platform: &PlatformClient,
    files: &[DiscordAttachment],
    config: &AttachmentConfig,
) -> Collected {
    let mut collected = Collected::default();
    if !config.enabled || files.is_empty() {
        return collected;
    }

    let described: Vec<IncomingFile> = files
        .iter()
        .map(|file| describe(&file.filename, file.content_type.as_deref(), file.size))
        .collect();
    for ((file, incoming), item) in files.iter().zip(&described).zip(plan(&described, config)) {
        let outcome = match item {
            PlanItem::Skip(reason) => Err(reason),
            PlanItem::Fetch => relay(platform, file, incoming, config).await,
        };
        match outcome {
            Ok(attachment) => collected.attachments.push(attachment),
            Err(reason) => {
                tracing::info!(
                    file = incoming.name,
                    %reason,
                    "discord connector: a file did not come through; announcing it instead"
                );
                collected.notes.push(reason.note(&incoming.name));
            }
        }
    }
    collected
}

/// Append the notes for the files that did not come through to a message's text, separated from it
/// by a blank line. A message whose only content was an excluded file still says so, so the agent
/// never sees an empty turn where a file was.
pub(super) fn append_notes(text: &str, notes: &[String]) -> String {
    if notes.is_empty() {
        return text.to_owned();
    }
    let notes = notes.join("\n");
    if text.trim().is_empty() {
        notes
    } else {
        format!("{text}\n\n{notes}")
    }
}

/// One file as Discord described it, before anything was fetched.
struct IncomingFile {
    name: String,
    mime: String,
    byte_len: u64,
}

/// What the connector does with one file, decided before any byte is fetched.
enum PlanItem {
    /// Fetch the bytes and upload them.
    Fetch,
    /// Excluded by a cap; announce it instead.
    Skip(NotDelivered),
}

/// Why a file did not reach the agent. Each variant renders the note the agent reads, so a failure
/// mode is named in plain language rather than left to a log line the agent never sees.
enum NotDelivered {
    /// The file is larger than the connector's per-file cap.
    TooLarge { byte_len: u64, cap: u64 },
    /// The message carried more files than the connector's per-message cap.
    TooMany { cap: usize },
    /// The download from Discord failed, or ran past the fetch timeout.
    FetchFailed,
    /// The upload to the agent's blob store failed.
    UploadFailed,
}

impl NotDelivered {
    /// The note the agent reads for this file, naming what was shared and why it is not here.
    fn note(&self, name: &str) -> String {
        match self {
            NotDelivered::TooLarge { byte_len, cap } => format!(
                "({name} was shared, but it did not come through: it is {byte_len} bytes, over this connector's {cap}-byte limit)"
            ),
            NotDelivered::TooMany { cap } => format!(
                "({name} was shared, but it did not come through: the message carried more files than this connector relays at once, which is {cap})"
            ),
            NotDelivered::FetchFailed => {
                format!("({name} was shared, but it could not be downloaded from Discord)")
            }
            NotDelivered::UploadFailed => {
                format!("({name} was shared, but it could not be stored for the agent to read)")
            }
        }
    }
}

impl fmt::Display for NotDelivered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotDelivered::TooLarge { byte_len, cap } => {
                write!(f, "{byte_len} bytes, over the {cap}-byte limit")
            }
            NotDelivered::TooMany { cap } => write!(f, "over the {cap}-file limit for one message"),
            NotDelivered::FetchFailed => write!(f, "the download from Discord failed"),
            NotDelivered::UploadFailed => write!(f, "the upload to the blob store failed"),
        }
    }
}

/// The media type a file with no usable reported one is treated as when its filename is not in the
/// conservative fallback table.
const OCTET_STREAM: &str = "application/octet-stream";

/// Read Discord's description of one file. Explicit non-generic media types remain authoritative;
/// absent, blank, and generic metadata uses only the conservative filename table below. Extensions are
/// never proof of the bytes' contents: text still has to decode as UTF-8, and unknown or active-content
/// extensions remain opaque.
fn describe(name: &str, content_type: Option<&str>, byte_len: u32) -> IncomingFile {
    let reported = content_type.map(str::trim).filter(|mime| !mime.is_empty());
    let mime = reported
        .filter(|mime| !is_generic_mime(mime))
        .map(str::to_owned)
        .or_else(|| fallback_mime(name).map(str::to_owned))
        .unwrap_or_else(|| OCTET_STREAM.to_owned());
    IncomingFile {
        name: name.to_owned(),
        mime,
        byte_len: u64::from(byte_len),
    }
}

/// Whether the media-type token before parameters is the generic Discord placeholder.
fn is_generic_mime(mime: &str) -> bool {
    mime.split(';')
        .next()
        .is_some_and(|token| token.trim().eq_ignore_ascii_case(OCTET_STREAM))
}

/// The deliberately narrow filename fallback for files Discord labels as absent or generic.
fn fallback_mime(name: &str) -> Option<&'static str> {
    let extension = name.rsplit('.').next()?.to_ascii_lowercase();
    match extension.as_str() {
        "txt" | "py" | "rs" | "json" | "log" | "md" | "csv" | "toml" | "yaml" | "yml" | "ini"
        | "cfg" | "conf" | "c" | "h" | "cc" | "cpp" | "hpp" | "java" | "js" | "jsx" | "ts"
        | "tsx" | "go" | "rb" | "sh" | "sql" | "css" => Some("text/plain"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Decide each file's fate against the caps, in the order the message carried them. The per-message
/// cap counts only the files that would actually be fetched, so a run of oversized files does not
/// consume the budget for the ones that fit.
fn plan(files: &[IncomingFile], config: &AttachmentConfig) -> Vec<PlanItem> {
    let mut fetched = 0;
    files
        .iter()
        .map(|file| {
            if file.byte_len > config.max_bytes {
                return PlanItem::Skip(NotDelivered::TooLarge {
                    byte_len: file.byte_len,
                    cap: config.max_bytes,
                });
            }
            if fetched >= config.max_per_message {
                return PlanItem::Skip(NotDelivered::TooMany {
                    cap: config.max_per_message,
                });
            }
            fetched += 1;
            PlanItem::Fetch
        })
        .collect()
}
