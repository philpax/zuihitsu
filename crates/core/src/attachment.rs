//! The attachment record: what a platform message carries alongside its text, and what the event log
//! records verbatim on the turn.
//!
//! Only the *record* lives here — the name, the media type, the content address, the size, and the
//! classification the rest of the system branches on. The bytes live in the host's blob store, keyed
//! by [`BlobHash`], because the log is the source of truth and stays small and replayable while blob
//! bytes are bulk and immutable.
//!
//! [`AttachmentKind::of_mime`] is the single classification: the connector labels an upload with it,
//! the server re-derives it when a message names a blob, and the console renders from it, so all
//! three agree on whether a given media type is perceivable, inlinable, or merely announced.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::BlobHash;

/// One file riding an inbound message: the sender's name for it, the media type it was uploaded
/// under, the content address its bytes are stored at, their length, and how the turn should treat
/// it. Recorded on the participant's `ConversationTurn`, so a replayed turn carries exactly the
/// attachments the live one saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Attachment {
    /// The sender's name for the file, as the platform reported it. Display only — it is untrusted
    /// text, never a path and never an identity.
    pub name: String,
    /// The media type the bytes were stored under, verbatim from the upload.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub mime: SmolStr,
    /// The content address the bytes are stored at in the blob store.
    pub blob: BlobHash,
    /// The stored bytes' length.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub byte_len: u64,
    /// How the turn treats the attachment, derived from `mime` by [`AttachmentKind::of_mime`].
    pub kind: AttachmentKind,
}

/// How an attachment reaches the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum AttachmentKind {
    /// The model can perceive it: it rides the request as an image content part.
    Image,
    /// UTF-8 text: it is inlined into the message body at assembly.
    Text,
    /// Anything else: announced by name, type, and size, and never inlined.
    Opaque,
}

impl AttachmentKind {
    /// Classify a media type. The comparison is over the type itself, so a `; charset=utf-8`
    /// parameter and an upper-case spelling classify as the bare lower-case type does.
    pub fn of_mime(mime: &str) -> AttachmentKind {
        let media_type = mime
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match media_type.as_str() {
            // The web image types a vision model takes as an image content part. A type outside this
            // set is opaque even when it is an image: the model cannot decode it.
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" => AttachmentKind::Image,
            // The structured-text types that are text without saying so in their type.
            "application/json" | "application/xml" | "application/x-yaml" | "application/toml" => {
                AttachmentKind::Text
            }
            other if other.starts_with("text/") => AttachmentKind::Text,
            // The structured-syntax suffix (RFC 6838 §4.2.8): `application/ld+json`,
            // `image/svg+xml`, and every other vendor type built on one of the two.
            other if other.ends_with("+json") || other.ends_with("+xml") => AttachmentKind::Text,
            _ => AttachmentKind::Opaque,
        }
    }
}

#[cfg(test)]
mod tests {
    //! The classification contract the connector, the server, and the console all read: which media
    //! types the model can perceive, which are text, and that anything unrecognised stays opaque.

    use super::AttachmentKind::{self, Image, Opaque, Text};

    #[test]
    fn a_media_type_classifies() {
        let cases = [
            ("image/png", Image),
            ("image/jpeg", Image),
            ("image/gif", Image),
            ("image/webp", Image),
            // An image the model cannot decode is opaque, not an image content part.
            ("image/tiff", Opaque),
            ("text/plain", Text),
            ("text/markdown", Text),
            ("application/json", Text),
            ("application/xml", Text),
            ("application/x-yaml", Text),
            ("application/toml", Text),
            // The structured-syntax suffix rule, in both spellings.
            ("application/ld+json", Text),
            ("image/svg+xml", Text),
            ("application/vnd.example.thing+json", Text),
            ("application/pdf", Opaque),
            ("application/octet-stream", Opaque),
            ("audio/mpeg", Opaque),
            // An unknown type, and a type-shaped string that is not one at all.
            ("application/x-invented-by-nobody", Opaque),
            ("", Opaque),
        ];
        for (mime, expected) in cases {
            assert_eq!(
                AttachmentKind::of_mime(mime),
                expected,
                "classifying {mime}"
            );
        }
    }

    #[test]
    fn parameters_and_case_do_not_change_the_classification() {
        assert_eq!(
            AttachmentKind::of_mime("text/plain; charset=utf-8"),
            AttachmentKind::Text
        );
        assert_eq!(
            AttachmentKind::of_mime("  IMAGE/PNG  "),
            AttachmentKind::Image
        );
        assert_eq!(
            AttachmentKind::of_mime("Application/LD+JSON"),
            AttachmentKind::Text
        );
    }
}
