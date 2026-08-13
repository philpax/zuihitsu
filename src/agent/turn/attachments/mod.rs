//! Rendering a turn's attachments into the prompt: what the model reads about a file, and which
//! files it actually sees.
//!
//! A message's attachments reach the model three ways, by kind. Text is inlined into the message body
//! so the agent can read it. An image becomes an [`ImagePart`] the request carries as a content part,
//! so the model perceives it directly. Anything else is announced — name, media type, size — so the
//! agent knows something was shared and that it cannot read it, rather than being left to guess from
//! a message whose text alone says "have a look at this".
//!
//! **Rendering is a pure function of the recorded record and the stored bytes.** The blob store is
//! content-addressed and the log records the attachment verbatim, so a turn replayed from the buffer
//! renders byte-identically to the turn that first sent it, and the serving layer's prefix cache
//! survives the replay. Nothing here reads the clock, the graph, or any setting that could change
//! between the two.

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::{
    attachment::{Attachment, AttachmentKind},
    model::ImagePart,
    store::BlobStore,
};

/// A message body with its attachments rendered in, and the images the request should carry.
pub(crate) struct RenderedAttachments {
    /// The message text with each attachment's inlined content or announcement appended.
    pub body: String,
    /// One part per image the model can perceive, in the order the attachments were carried.
    pub images: Vec<ImagePart>,
}

/// Render `attachments` against `text`, reading their bytes from `blobs`.
///
/// `max_text_chars` is the message's total inlining budget, not each file's, spent in carry order —
/// otherwise the per-file cap multiplies by the per-message file count. A file that cannot be read,
/// or that arrives after the budget is gone, is announced with the reason.
pub(crate) fn render(
    text: &str,
    attachments: &[Attachment],
    blobs: &BlobStore,
    max_text_chars: usize,
) -> RenderedAttachments {
    let mut body = text.to_owned();
    let mut images = Vec::new();
    let mut text_budget = max_text_chars;
    for attachment in attachments {
        let rendered = match attachment.kind {
            AttachmentKind::Text => {
                let (rendered, spent) = inline_text(attachment, blobs, text_budget);
                text_budget -= spent;
                rendered
            }
            AttachmentKind::Image => match read(attachment, blobs) {
                Read::Bytes(bytes) => {
                    images.push(ImagePart {
                        blob: attachment.blob.clone(),
                        mime: attachment.mime.clone(),
                        data: STANDARD.encode(&bytes).into(),
                    });
                    announce(attachment, "shown below")
                }
                Read::Missing => announce(attachment, "an image, but its content is unavailable"),
                Read::Failed => announce(attachment, "an image, but it could not be read"),
            },
            AttachmentKind::Opaque => announce(attachment, "not something you can read"),
        };
        body.push_str("\n\n");
        body.push_str(&rendered);
    }
    RenderedAttachments { body, images }
}

/// What reading an attachment's bytes yielded. A store that failed is not the same as a store that
/// never held these bytes: the first is an operational fault worth a log line, the second is the
/// ordinary consequence of a collected or reverted blob.
enum Read {
    Bytes(Vec<u8>),
    Missing,
    Failed,
}

/// Read one attachment's bytes, logging a backend failure. The turn proceeds either way — a prompt
/// that announces what it cannot read beats a turn that does not run.
fn read(attachment: &Attachment, blobs: &BlobStore) -> Read {
    match blobs.get(&attachment.blob) {
        Ok(Some(blob)) => Read::Bytes(blob.bytes),
        Ok(None) => Read::Missing,
        Err(error) => {
            tracing::warn!(
                %error,
                blob = %attachment.blob,
                name = attachment.name,
                "could not read an attachment's bytes; announcing it instead"
            );
            Read::Failed
        }
    }
}

/// One attachment's one-line announcement: what it is called, what it is, how big, and why it reads
/// the way it does.
fn announce(attachment: &Attachment, note: &str) -> String {
    format!(
        "[attachment: {} ({}, {} bytes) — {note}]",
        attachment.name, attachment.mime, attachment.byte_len
    )
}

/// A text attachment's announcement and as much content as `budget` allows, with what it spent. A
/// file that does not inline spends nothing.
fn inline_text(attachment: &Attachment, blobs: &BlobStore, budget: usize) -> (String, usize) {
    let bytes = match read(attachment, blobs) {
        Read::Bytes(bytes) => bytes,
        Read::Missing => {
            return (
                announce(attachment, "text, but its content is unavailable"),
                0,
            );
        }
        Read::Failed => return (announce(attachment, "text, but it could not be read"), 0),
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return (announce(attachment, "not decodable as text"), 0);
    };
    if budget == 0 {
        return (
            announce(
                attachment,
                "text, but the message's earlier files used its whole inlining budget",
            ),
            0,
        );
    }
    let (shown, clipped) = clip(&text, budget);
    let spent = shown.chars().count();
    let note = if clipped {
        format!("text, showing the first {spent} characters")
    } else {
        "text, shown in full".to_owned()
    };
    // The fence is longer than the longest backtick run the content holds, so content that is itself
    // fenced Markdown cannot close the fence early.
    let fence = "`".repeat(longest_backtick_run(shown).max(2) + 1);
    (
        format!("{}\n{fence}\n{shown}\n{fence}", announce(attachment, &note)),
        spent,
    )
}

/// `text` truncated to at most `max_chars` characters, and whether truncation happened. Counts
/// Unicode scalar values, so the cut never lands inside a character.
fn clip(text: &str, max_chars: usize) -> (&str, bool) {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => (&text[..cut], true),
        None => (text, false),
    }
}

/// The length of the longest run of consecutive backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

#[cfg(test)]
mod tests;
