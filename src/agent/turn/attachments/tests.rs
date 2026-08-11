//! What the model reads about an attachment, and what it is shown.

use crate::{
    agent::turn::attachments::render,
    attachment::{Attachment, AttachmentKind},
    store::BlobStore,
};

/// Store `bytes` and describe them the way the platform handler does — the store is authoritative
/// for the media type, length, and kind.
fn attach(blobs: &BlobStore, name: &str, mime: &str, bytes: &[u8]) -> Attachment {
    let blob = blobs.put(bytes, mime).unwrap();
    Attachment {
        name: name.to_owned(),
        mime: mime.into(),
        blob,
        byte_len: bytes.len() as u64,
        kind: AttachmentKind::of_mime(mime),
    }
}

#[test]
fn a_message_with_no_attachments_is_left_exactly_as_it_was() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let rendered = render("just words", &[], &blobs, 100);
    assert_eq!(rendered.body, "just words");
    assert!(rendered.images.is_empty());
}

#[test]
fn a_text_attachment_is_inlined_under_its_announcement() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let attachment = attach(&blobs, "notes.txt", "text/plain", b"line one\nline two");
    let rendered = render("have a look", &[attachment], &blobs, 100);

    assert_eq!(
        rendered.body,
        "have a look\n\n\
         [attachment: notes.txt (text/plain, 17 bytes) — text, shown in full]\n\
         ```\nline one\nline two\n```"
    );
    assert!(rendered.images.is_empty());
}

#[test]
fn a_long_text_attachment_is_clipped_and_says_so() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let attachment = attach(&blobs, "big.log", "text/plain", &b"x".repeat(50));
    let rendered = render("", &[attachment], &blobs, 10);

    assert!(
        rendered
            .body
            .contains("text, showing the first 10 characters]"),
        "{}",
        rendered.body
    );
    assert!(rendered.body.contains(&"x".repeat(10)));
    assert!(!rendered.body.contains(&"x".repeat(11)));
}

#[test]
fn fenced_content_cannot_close_the_fence_early() {
    let blobs = BlobStore::open_in_memory().unwrap();
    // The file is itself fenced Markdown; a three-backtick fence around it would end at its first
    // fence and spill the rest into the conversation as prose.
    let attachment = attach(
        &blobs,
        "readme.md",
        "text/markdown",
        b"```rust\nfn main() {}\n```",
    );
    let rendered = render("", &[attachment], &blobs, 1_000);

    assert!(rendered.body.contains("````\n```rust"), "{}", rendered.body);
    assert!(rendered.body.ends_with("```\n````"), "{}", rendered.body);
}

#[test]
fn clipping_never_splits_a_character() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let attachment = attach(&blobs, "kana.txt", "text/plain", "ひらがな".as_bytes());
    // Two characters of a four-character string that is twelve bytes long: a byte-wise cut here
    // would land mid-character and panic.
    let rendered = render("", &[attachment], &blobs, 2);
    assert!(rendered.body.contains("ひら"), "{}", rendered.body);
    assert!(!rendered.body.contains("ひらが"), "{}", rendered.body);
}

#[test]
fn an_image_is_announced_and_carried_as_a_part() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let attachment = attach(&blobs, "shot.png", "image/png", b"\x89PNG\r\n\x1a\n");
    let rendered = render(
        "what is this",
        std::slice::from_ref(&attachment),
        &blobs,
        100,
    );

    assert_eq!(
        rendered.body,
        "what is this\n\n[attachment: shot.png (image/png, 8 bytes) — shown below]"
    );
    let [image] = &rendered.images[..] else {
        panic!("one image part, got {:?}", rendered.images);
    };
    assert_eq!(image.blob, attachment.blob);
    assert_eq!(image.mime, "image/png");
    // Base64 of the PNG magic bytes — what the `data:` URI carries to the backend.
    assert_eq!(&*image.data, "iVBORw0KGgo=");
}

#[test]
fn an_unreadable_attachment_is_announced_rather_than_failing_the_turn() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let archive = attach(&blobs, "bundle.zip", "application/zip", b"PK\x03\x04");
    let invalid = attach(&blobs, "broken.txt", "text/plain", &[0xff, 0xfe, 0xfd]);
    let rendered = render("", &[archive, invalid], &blobs, 100);

    assert!(
        rendered.body.contains(
            "[attachment: bundle.zip (application/zip, 4 bytes) — not something you can read]"
        ),
        "{}",
        rendered.body
    );
    assert!(
        rendered
            .body
            .contains("[attachment: broken.txt (text/plain, 3 bytes) — not decodable as text]"),
        "{}",
        rendered.body
    );
    assert!(rendered.images.is_empty());
}

#[test]
fn an_image_whose_bytes_are_gone_degrades_instead_of_being_shown() {
    let blobs = BlobStore::open_in_memory().unwrap();
    let attachment = attach(&blobs, "shot.png", "image/png", b"\x89PNG");
    blobs.remove(&attachment.blob).unwrap();
    let rendered = render("", &[attachment], &blobs, 100);

    assert!(
        rendered.body.contains("its content is unavailable"),
        "{}",
        rendered.body
    );
    assert!(rendered.images.is_empty());
}

#[test]
fn rendering_the_same_record_twice_is_byte_identical() {
    // The prefix cache depends on this: a turn replayed out of the buffer must render exactly as the
    // turn that first sent it, or every subsequent turn re-prefills.
    let blobs = BlobStore::open_in_memory().unwrap();
    let attachments = vec![
        attach(&blobs, "notes.txt", "text/plain", b"some notes"),
        attach(&blobs, "shot.png", "image/png", b"\x89PNG"),
        attach(&blobs, "bundle.zip", "application/zip", b"PK"),
    ];
    let first = render("look", &attachments, &blobs, 100);
    let second = render("look", &attachments, &blobs, 100);

    assert_eq!(first.body, second.body);
    assert_eq!(first.images, second.images);
}
