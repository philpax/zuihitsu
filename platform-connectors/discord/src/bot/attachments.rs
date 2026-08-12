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

use zuihitsu_core::ids::BlobHash;
use zuihitsu_platform_connector_api::{Error as PlatformError, MessageAttachment, PlatformClient};

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

#[async_trait::async_trait]
trait AttachmentDownloader: Send + Sync {
    async fn download(&self, url: &str) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Debug)]
enum UploadFailure {
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
    Other(String),
}

impl fmt::Display for UploadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadFailure::Status { status, body } => write!(f, "upload returned {status}: {body}"),
            UploadFailure::Other(message) => f.write_str(message),
        }
    }
}

#[async_trait::async_trait]
trait BlobUploader: Send + Sync {
    async fn upload(&self, bytes: Vec<u8>, mime: &str) -> Result<BlobHash, UploadFailure>;
}

struct DiscordDownloader {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl AttachmentDownloader for DiscordDownloader {
    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }
}

struct PlatformUploader<'a> {
    platform: &'a PlatformClient,
}

#[async_trait::async_trait]
impl BlobUploader for PlatformUploader<'_> {
    async fn upload(&self, bytes: Vec<u8>, mime: &str) -> Result<BlobHash, UploadFailure> {
        self.platform
            .upload_blob(bytes, mime)
            .await
            .map_err(|error| match error {
                PlatformError::Status { status, body, .. } => {
                    UploadFailure::Status { status, body }
                }
                other => UploadFailure::Other(other.to_string()),
            })
    }
}

/// Fetch and store one file, with the download and upload operations injectable for tests. The size is
/// re-checked against the cap after the download, since Discord's reported size is a claim about bytes
/// we had not yet seen.
async fn relay_with<D: AttachmentDownloader, U: BlobUploader>(
    downloader: &D,
    uploader: &U,
    url: &str,
    incoming: &IncomingFile,
    config: &AttachmentConfig,
) -> Result<MessageAttachment, NotDelivered> {
    let bytes = match timeout(FETCH_TIMEOUT, downloader.download(url)).await {
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
    match uploader.upload(bytes, &incoming.mime).await {
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

/// Fetch one Discord file with status-aware HTTP handling, then upload it to the platform.
async fn relay(
    platform: &PlatformClient,
    file: &DiscordAttachment,
    incoming: &IncomingFile,
    config: &AttachmentConfig,
) -> Result<MessageAttachment, NotDelivered> {
    let downloader = DiscordDownloader {
        client: reqwest::Client::new(),
    };
    let uploader = PlatformUploader { platform };
    relay_with(&downloader, &uploader, &file.url, incoming, config).await
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
    fn describe_uses_only_the_conservative_filename_mime_fallback_table() {
        let text_extensions = [
            "txt", "py", "rs", "json", "log", "md", "csv", "toml", "yaml", "yml", "ini", "cfg",
            "conf", "c", "h", "cc", "cpp", "hpp", "java", "js", "jsx", "ts", "tsx", "go", "rb",
            "sh", "sql", "css",
        ];
        for extension in text_extensions {
            assert_eq!(
                describe(&format!("file.{extension}"), None, 10).mime,
                "text/plain"
            );
        }
        for (extension, expected) in [
            ("png", "image/png"),
            ("jpg", "image/jpeg"),
            ("jpeg", "image/jpeg"),
            ("gif", "image/gif"),
            ("webp", "image/webp"),
        ] {
            assert_eq!(
                describe(&format!("file.{extension}"), None, 10).mime,
                expected
            );
        }
        assert_eq!(describe("notes.TXT", None, 10).mime, "text/plain");
        assert_eq!(
            describe("src/main.Rs", Some("APPLICATION/OCTET-STREAM"), 10).mime,
            "text/plain"
        );
        assert_eq!(
            describe(
                "photo.JpEg",
                Some(" application/octet-stream ; foo=bar "),
                10
            )
            .mime,
            "image/jpeg"
        );
        assert_eq!(
            describe("shot.png", Some("image/png"), 10).mime,
            "image/png"
        );
        assert_eq!(
            describe("notes.txt", Some("text/markdown; charset=utf-8"), 10).mime,
            "text/markdown; charset=utf-8"
        );
        for name in [
            "mystery.bin",
            "page.html",
            "page.htm",
            "vector.xml",
            "vector.svg",
            "no-extension",
        ] {
            assert_eq!(describe(name, None, 10).mime, OCTET_STREAM);
        }
    }

    struct TestDownloader {
        result: Result<Vec<u8>, String>,
    }

    #[async_trait::async_trait]
    impl AttachmentDownloader for TestDownloader {
        async fn download(&self, _url: &str) -> Result<Vec<u8>, String> {
            self.result.clone()
        }
    }

    struct TestUploader {
        calls: std::sync::atomic::AtomicUsize,
        result: Result<BlobHash, UploadFailure>,
    }

    #[async_trait::async_trait]
    impl BlobUploader for TestUploader {
        async fn upload(&self, _bytes: Vec<u8>, _mime: &str) -> Result<BlobHash, UploadFailure> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.result.clone()
        }
    }

    #[tokio::test]
    async fn non_success_discord_response_is_fetch_failure_and_skips_upload() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 10\r\n\r\nnot a file")
                .await
                .unwrap();
        });

        let downloader = DiscordDownloader {
            client: reqwest::Client::new(),
        };
        let uploader = TestUploader {
            calls: std::sync::atomic::AtomicUsize::new(0),
            result: Ok(BlobHash::of(b"unexpected")),
        };
        let result = relay_with(
            &downloader,
            &uploader,
            &format!("http://{address}/attachment"),
            &describe("report.txt", None, 12),
            &config(100, 1),
        )
        .await;
        server.await.unwrap();
        assert!(matches!(result, Err(NotDelivered::FetchFailed)));
        assert_eq!(uploader.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn upload_conflict_is_announced_without_an_attachment() {
        let downloader = TestDownloader {
            result: Ok(b"same bytes".to_vec()),
        };
        let uploader = TestUploader {
            calls: std::sync::atomic::AtomicUsize::new(0),
            result: Err(UploadFailure::Status {
                status: reqwest::StatusCode::CONFLICT,
                body: "MIME conflict".to_owned(),
            }),
        };
        let result = relay_with(
            &downloader,
            &uploader,
            "https://discord.invalid/file",
            &describe("report.txt", None, 10),
            &config(100, 1),
        )
        .await;
        assert!(matches!(result, Err(NotDelivered::UploadFailed)));
        assert_eq!(uploader.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let note = NotDelivered::UploadFailed.note("report.txt");
        assert!(note.contains("report.txt"));
        assert!(!note.contains(BlobHash::of(b"same bytes").as_str()));
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
