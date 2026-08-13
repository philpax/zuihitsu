//! What the connector decides about a message's files, and what it does with the ones it fetches.

use zuihitsu_core::ids::BlobHash;

use super::{
    relay::{AttachmentDownloader, BlobUploader, DiscordDownloader, UploadFailure, relay_with},
    *,
};

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
        "txt", "py", "rs", "json", "log", "md", "csv", "toml", "yaml", "yml", "ini", "cfg", "conf",
        "c", "h", "cc", "cpp", "hpp", "java", "js", "jsx", "ts", "tsx", "go", "rb", "sh", "sql",
        "css",
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

#[test]
fn a_batch_whose_files_the_agent_refuses_is_delivered_as_text_with_a_note() {
    use crate::bot::process::without_attachments;
    use zuihitsu_core::ids::PersonId;
    use zuihitsu_platform_connector_api::{MessageAttachment, PlatformMessage};

    let carried = PlatformMessage {
        sender: PersonId::new("discord", "rowan"),
        text: "here it is".to_owned(),
        attachments: vec![MessageAttachment {
            name: "build.log".to_owned(),
            blob: BlobHash::of(b"log bytes"),
        }],
    };
    let plain = PlatformMessage {
        sender: PersonId::new("discord", "rowan"),
        text: "and a thought".to_owned(),
        attachments: Vec::new(),
    };

    let [carried, plain] = &without_attachments(vec![carried, plain])[..] else {
        panic!("the batch keeps its messages");
    };
    assert!(carried.attachments.is_empty());
    assert_eq!(
        carried.text,
        "here it is\n\n(build.log was shared, but it could not be delivered to you)"
    );
    // A message that carried nothing is untouched.
    assert_eq!(plain.text, "and a thought");
}
