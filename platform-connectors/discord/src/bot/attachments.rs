//! Relaying the files a Discord message carried: what to fetch, what to upload, and what to say
//! about whatever did not come through.
//!
//! The two halves are kept apart deliberately. [`plan`] decides — from the names, media types, and
//! sizes Discord reports, and the connector's caps — which files are worth fetching and which are
//! excluded before a byte moves; [`collect`] then drives the download and the blob upload for the
//! ones that survived. Every exclusion and every failure produces a note appended to the message
//! text, because the agent knowing that `build.log` was shared and did not arrive is the point: a
//! silently dropped file reads to the agent as a message that carried nothing.

use serenity::all::Attachment as DiscordAttachment;
use std::{fmt, time::Duration};
use tokio::time::timeout;

use zuihitsu_platform_connector_api::{MessageAttachment, PlatformClient};

use crate::config::AttachmentConfig;

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

/// How long one file's download may take before it is abandoned. A message is a live conversational
/// turn, so a stalled download must not hold the whole batch: past this the file is announced rather
/// than waited on.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// The media type a file with no reported one is treated as — "some bytes", which is what Discord
/// actually said about it.
const OCTET_STREAM: &str = "application/octet-stream";

/// Read Discord's description of one file: its name, the media type it reported (or the generic type
/// when it reported none), and its size.
fn describe(name: &str, content_type: Option<&str>, byte_len: u32) -> IncomingFile {
    let mime = content_type
        .map(str::trim)
        .filter(|mime| !mime.is_empty())
        .unwrap_or(OCTET_STREAM)
        .to_owned();
    IncomingFile {
        name: name.to_owned(),
        mime,
        byte_len: u64::from(byte_len),
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

/// Fetch one file's bytes and store them, yielding the record that names the stored blob. The size
/// is re-checked against the cap after the download, since Discord's reported size is a claim about
/// bytes we had not yet seen.
async fn relay(
    platform: &PlatformClient,
    file: &DiscordAttachment,
    incoming: &IncomingFile,
    config: &AttachmentConfig,
) -> Result<MessageAttachment, NotDelivered> {
    let bytes = match timeout(FETCH_TIMEOUT, file.download()).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            tracing::warn!(%error, file = incoming.name, "discord connector: could not download a file");
            return Err(NotDelivered::FetchFailed);
        }
        Err(_) => {
            tracing::warn!(
                file = incoming.name,
                timeout_secs = FETCH_TIMEOUT.as_secs(),
                "discord connector: a file download timed out"
            );
            return Err(NotDelivered::FetchFailed);
        }
    };
    let byte_len = bytes.len() as u64;
    if byte_len > config.max_bytes {
        return Err(NotDelivered::TooLarge {
            byte_len,
            cap: config.max_bytes,
        });
    }
    match platform.upload_blob(bytes, &incoming.mime).await {
        Ok(blob) => Ok(MessageAttachment {
            name: incoming.name.clone(),
            blob,
        }),
        Err(error) => {
            tracing::warn!(%error, file = incoming.name, "discord connector: could not upload a file");
            Err(NotDelivered::UploadFailed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(max_bytes: u64, max_per_message: usize) -> AttachmentConfig {
        AttachmentConfig {
            enabled: true,
            max_bytes,
            max_per_message,
        }
    }

    fn fates(files: &[IncomingFile], config: &AttachmentConfig) -> Vec<String> {
        plan(files, config)
            .into_iter()
            .zip(files)
            .map(|(item, file)| match item {
                PlanItem::Fetch => "fetch".to_owned(),
                PlanItem::Skip(reason) => reason.note(&file.name),
            })
            .collect()
    }

    #[test]
    fn a_file_discord_gives_no_type_for_is_uploaded_as_opaque_bytes() {
        // The media type is only what the upload is labelled with — the classification it implies is
        // the server's, derived from the stored blob, so the connector keeps no copy of that rule.
        assert_eq!(
            describe("shot.png", Some("image/png"), 10).mime,
            "image/png"
        );
        assert_eq!(describe("mystery.bin", None, 10).mime, OCTET_STREAM);
        // An empty type is no type at all, not a media type of "".
        assert_eq!(describe("mystery.bin", Some("  "), 10).mime, OCTET_STREAM);
    }

    #[test]
    fn a_file_over_the_byte_cap_is_announced_rather_than_fetched() {
        let files = [
            describe("small.txt", Some("text/plain"), 100),
            describe("build.log", Some("text/plain"), 300),
        ];
        assert_eq!(
            fates(&files, &config(200, 4)),
            [
                "fetch".to_owned(),
                "(build.log was shared, but it did not come through: it is 300 bytes, over this connector's 200-byte limit)".to_owned(),
            ]
        );
        // The cap is inclusive: a file exactly at it still comes through.
        assert_eq!(fates(&files[1..], &config(300, 4)), ["fetch".to_owned()]);
    }

    #[test]
    fn files_past_the_count_cap_are_announced_rather_than_fetched() {
        let files: Vec<IncomingFile> = ["a.txt", "b.txt", "c.txt"]
            .iter()
            .map(|name| describe(name, Some("text/plain"), 10))
            .collect();
        assert_eq!(
            fates(&files, &config(1000, 2)),
            [
                "fetch".to_owned(),
                "fetch".to_owned(),
                "(c.txt was shared, but it did not come through: the message carried more files than this connector relays at once, which is 2)".to_owned(),
            ]
        );
    }

    #[test]
    fn an_oversized_file_does_not_spend_the_count_budget() {
        // The count cap bounds the downloads, so a file excluded before any fetch must not push a
        // file that fits out of the budget.
        let files = [
            describe("huge.bin", Some("application/octet-stream"), 5000),
            describe("a.txt", Some("text/plain"), 10),
            describe("b.txt", Some("text/plain"), 10),
        ];
        let fates = fates(&files, &config(1000, 2));
        assert!(fates[0].starts_with("(huge.bin was shared"));
        assert_eq!(fates[1..], ["fetch".to_owned(), "fetch".to_owned()]);
    }

    #[test]
    fn every_failure_mode_names_the_file_and_its_reason() {
        assert_eq!(
            NotDelivered::FetchFailed.note("shot.png"),
            "(shot.png was shared, but it could not be downloaded from Discord)"
        );
        assert_eq!(
            NotDelivered::UploadFailed.note("shot.png"),
            "(shot.png was shared, but it could not be stored for the agent to read)"
        );
        assert_eq!(
            NotDelivered::TooLarge {
                byte_len: 20_971_520,
                cap: 16_777_216
            }
            .note("build.log"),
            "(build.log was shared, but it did not come through: it is 20971520 bytes, over this connector's 16777216-byte limit)"
        );
        assert_eq!(
            NotDelivered::TooMany { cap: 4 }.note("extra.png"),
            "(extra.png was shared, but it did not come through: the message carried more files than this connector relays at once, which is 4)"
        );
    }

    #[test]
    fn notes_ride_below_the_message_text() {
        let notes = vec![NotDelivered::FetchFailed.note("shot.png")];
        assert_eq!(
            append_notes("have a look", &notes),
            "have a look\n\n(shot.png was shared, but it could not be downloaded from Discord)"
        );
        // A message that was only a file is still a message: the note becomes its whole text rather
        // than arriving with a leading blank line.
        assert_eq!(append_notes("   ", &notes), notes[0]);
        assert_eq!(append_notes("have a look", &[]), "have a look");
    }
}
