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
/// A text attachment longer than `max_text_chars` is inlined up to the cap and marked as clipped, so
/// a large paste informs the turn without displacing the conversation. A blob that is missing or is
/// not decodable text degrades to the same announcement an opaque attachment gets: the turn proceeds
/// and the agent is told plainly what it cannot read.
pub(crate) fn render(
    text: &str,
    attachments: &[Attachment],
    blobs: &BlobStore,
    max_text_chars: usize,
) -> RenderedAttachments {
    let mut body = text.to_owned();
    let mut images = Vec::new();
    for attachment in attachments {
        let rendered = match attachment.kind {
            AttachmentKind::Text => inline_text(attachment, blobs, max_text_chars),
            AttachmentKind::Image => match blobs.get(&attachment.blob) {
                Ok(Some(blob)) => {
                    images.push(ImagePart {
                        blob: attachment.blob.clone(),
                        mime: attachment.mime.clone(),
                        data: STANDARD.encode(&blob.bytes).into(),
                    });
                    announce(attachment, "shown below")
                }
                _ => announce(attachment, "an image, but its content is unavailable"),
            },
            AttachmentKind::Opaque => announce(attachment, "not something you can read"),
        };
        body.push_str("\n\n");
        body.push_str(&rendered);
    }
    RenderedAttachments { body, images }
}

/// One attachment's one-line announcement: what it is called, what it is, how big, and why it reads
/// the way it does.
fn announce(attachment: &Attachment, note: &str) -> String {
    format!(
        "[attachment: {} ({}, {} bytes) — {note}]",
        attachment.name, attachment.mime, attachment.byte_len
    )
}

/// A text attachment's announcement followed by its content in a fence.
fn inline_text(attachment: &Attachment, blobs: &BlobStore, max_text_chars: usize) -> String {
    let Ok(Some(blob)) = blobs.get(&attachment.blob) else {
        return announce(attachment, "text, but its content is unavailable");
    };
    let Ok(text) = String::from_utf8(blob.bytes) else {
        return announce(attachment, "not decodable as text");
    };
    let (shown, clipped) = clip(&text, max_text_chars);
    let note = if clipped {
        format!("text, showing the first {max_text_chars} characters")
    } else {
        "text, shown in full".to_owned()
    };
    // The fence is longer than the longest backtick run the content holds, so content that is itself
    // fenced Markdown cannot close the fence early.
    let fence = "`".repeat(longest_backtick_run(shown).max(2) + 1);
    format!("{}\n{fence}\n{shown}\n{fence}", announce(attachment, &note))
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
