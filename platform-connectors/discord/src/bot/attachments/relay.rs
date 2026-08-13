//! Fetching one file's bytes and storing them for the agent: the download from Discord, the upload to
//! the blob store, and the seams that let a test drive both.

use std::{fmt, time::Duration};
use tokio::time::timeout;

use serenity::all::Attachment as DiscordAttachment;
use zuihitsu_core::ids::BlobHash;
use zuihitsu_platform_connector_api::{Error as PlatformError, MessageAttachment, PlatformClient};

use crate::{
    bot::attachments::{IncomingFile, NotDelivered},
    config::AttachmentConfig,
};

/// How long one file's download may take before it is abandoned. A message is a live conversational
/// turn, so a stalled download must not hold the whole batch: past this the file is announced rather
/// than waited on.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[async_trait::async_trait]
pub(super) trait AttachmentDownloader: Send + Sync {
    async fn download(&self, url: &str) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Debug)]
pub(super) enum UploadFailure {
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
pub(super) trait BlobUploader: Send + Sync {
    async fn upload(&self, bytes: Vec<u8>, mime: &str) -> Result<BlobHash, UploadFailure>;
}

pub(super) struct DiscordDownloader {
    pub(super) client: reqwest::Client,
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
pub(super) async fn relay_with<D: AttachmentDownloader, U: BlobUploader>(
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
pub(super) async fn relay(
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
