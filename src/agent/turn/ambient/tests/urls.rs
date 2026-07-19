//! Tests for the URL concern: the scheme-anchored extraction of links from prose, and the URL lead in
//! the rendered hint — firing on its own, staying silent when browsing is off, deduping, capping, and
//! ordering after the token line.

use std::collections::HashSet;

use super::{corpus, topic};
use crate::{
    agent::turn::ambient::{MAX_URLS, ambient_recall, render::render, url::extract_urls},
    ids::{MemoryId, TurnId},
    settings::AmbientSettings,
    turn_ref,
};

#[test]
fn extract_urls_strips_trailing_sentence_punctuation() {
    // A URL at a sentence end must not keep the full stop.
    assert_eq!(
        extract_urls("See https://example.com/path."),
        vec!["https://example.com/path".to_owned()]
    );
}

#[test]
fn extract_urls_bounds_a_url_in_parens_and_brackets() {
    // A wrapping pair bounds the URL: neither the closing bracket nor a trailing delimiter rides along.
    assert_eq!(
        extract_urls("(https://example.com)"),
        vec!["https://example.com".to_owned()]
    );
    assert_eq!(
        extract_urls("[https://example.com/a]"),
        vec!["https://example.com/a".to_owned()]
    );
    assert_eq!(
        extract_urls("<https://example.com>"),
        vec!["https://example.com".to_owned()]
    );
    assert_eq!(
        extract_urls("read \"https://example.com/x\" now"),
        vec!["https://example.com/x".to_owned()]
    );
}

#[test]
fn extract_urls_discards_a_bare_scheme_with_no_host() {
    assert!(extract_urls("http:// is not a link").is_empty());
    assert!(extract_urls("here: https://").is_empty());
}

#[test]
fn extract_urls_keeps_appearance_order() {
    assert_eq!(
        extract_urls("first http://a.example then https://b.example done"),
        vec![
            "http://a.example".to_owned(),
            "https://b.example".to_owned()
        ]
    );
}

#[test]
fn a_url_fires_the_hint_when_browsing_is_on() {
    // A message carrying a link but matching nothing lexically and citing no turn still surfaces a
    // hint: the URL line points at web.markdown, so a shared link is never inert.
    let graph = corpus(Vec::new());
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "Have a look at https://example.com/article for context.",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the URL fires the hint even with no lexical hit");
    assert!(hint.hits.is_empty(), "no lexical hit rode along");
    assert!(
        hint.message
            .contains("web.markdown(\"https://example.com/article\")"),
        "the hint points at reading the link: {}",
        hint.message
    );
}

#[test]
fn a_url_is_silent_when_browsing_is_off() {
    // The web.markdown tool is browsing-gated, so with the feature off a URL yields no line — and,
    // with no lexical hit or token either, no hint at all.
    let graph = corpus(Vec::new());
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "Have a look at https://example.com/article for context.",
        &HashSet::new(),
        true,
        false,
    )
    .unwrap();
    assert!(
        hint.is_none(),
        "no URL line and no lexical hit means no hint"
    );
}

#[test]
fn a_repeated_url_gets_one_line() {
    let urls = vec!["https://example.com/a".to_owned()];
    let out = render(&[], &[], &urls, &[]);
    let url_lines = out.lines().filter(|l| l.contains("web.markdown")).count();
    assert_eq!(url_lines, 1, "one line per distinct URL");

    // And the dedup happens in the pass: a message repeating one URL surfaces one line.
    let graph = corpus(Vec::new());
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "See https://example.com/a and again https://example.com/a please.",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the URL fires the hint");
    assert_eq!(
        hint.message
            .lines()
            .filter(|l| l.contains("web.markdown"))
            .count(),
        1,
        "the repeated URL yields one line: {}",
        hint.message
    );
}

#[test]
fn the_url_lead_caps_at_the_first_few() {
    // A message carrying many links names only the first MAX_URLS, so the lead-in stays terse.
    let graph = corpus(Vec::new());
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "Three links: https://a.example https://b.example https://c.example.",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the URLs fire the hint");
    let url_lines = hint
        .message
        .lines()
        .filter(|l| l.contains("web.markdown"))
        .count();
    assert_eq!(url_lines, MAX_URLS, "the URL lead-in is capped");
}

#[test]
fn a_url_line_follows_the_token_and_precedes_the_lexical_block() {
    // With a token, a URL, and a salient lexical hit, the order is: token line, then URL line, then
    // the "possibly relevant" block.
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built; it versions and applies database migrations.",
    ));
    let turn = TurnId::generate();
    let message = format!(
        "What do you think of bonsai, given {} and https://example.com/notes?",
        turn_ref::construct(turn)
    );
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("a token, a URL, and a lexical hit surface");
    let token_line = hint
        .message
        .lines()
        .position(|l| l.contains("convo.turn"))
        .expect("a token line");
    let url_line = hint
        .message
        .lines()
        .position(|l| l.contains("web.markdown"))
        .expect("a URL line");
    let lexical_line = hint
        .message
        .lines()
        .position(|l| l.contains("Possibly relevant"))
        .expect("the lexical block");
    assert!(
        token_line < url_line && url_line < lexical_line,
        "token, then URL, then lexical: {}",
        hint.message
    );
}
