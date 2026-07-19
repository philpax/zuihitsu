//! Tests for the hint-rendering concern: the shape of a hit line, a URL line, and a decoded mem line,
//! independent of the recall orchestration.

use crate::{
    agent::turn::ambient::render::{ResolvedHit, ResolvedMem, render},
    ids::{MemoryId, MemoryName},
};

#[test]
fn render_writes_one_line_per_hit_and_no_empty_header() {
    let hits = vec![
        ResolvedHit {
            name: MemoryName::new("topic/bonsai"),
            snippet: "a schema-migration tool".to_owned(),
        },
        ResolvedHit {
            name: MemoryName::new("topic/driftwood"),
            snippet: String::new(),
        },
    ];
    let out = render(&[], &[], &[], &hits);
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("- ")).collect();
    assert_eq!(lines.len(), 2, "one line per hit");
    assert!(lines[0].contains("topic/bonsai") && lines[0].contains("schema-migration"));
    // An empty snippet renders the handle alone, with no dangling quotes.
    assert_eq!(lines[1], "- topic/driftwood");
}

#[test]
fn render_writes_a_url_line_pointing_at_web_markdown() {
    let urls = vec![
        "https://example.com/a".to_owned(),
        "https://example.com/b".to_owned(),
    ];
    let out = render(&[], &[], &urls, &[]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "one line per URL, no header");
    assert_eq!(
        lines[0],
        "The message includes a link — read it with web.markdown(\"https://example.com/a\")."
    );
    assert_eq!(
        lines[1],
        "The message includes a link — read it with web.markdown(\"https://example.com/b\")."
    );
}

#[test]
fn render_writes_a_mem_line_decoding_the_token() {
    let token = MemoryId::generate();
    let mems = vec![ResolvedMem {
        token,
        name: MemoryName::new("person/rowan@chat"),
    }];
    let out = render(&mems, &[], &[], &[]);
    assert_eq!(
        out,
        format!(
            "[mem:{}] refers to person/rowan@chat — read it with \
             memory.get(\"person/rowan@chat\") if useful.",
            token.0
        )
    );
}

#[test]
fn render_names_a_self_reference_without_a_read_suggestion() {
    // The reserved `self` memory is already in context, so its decode line names the agent itself
    // rather than suggesting a redundant read.
    let token = MemoryId::generate();
    let mems = vec![ResolvedMem {
        token,
        name: MemoryName::self_handle(),
    }];
    let out = render(&mems, &[], &[], &[]);
    assert_eq!(
        out,
        format!("[mem:{}] refers to self — that is you.", token.0)
    );
}
