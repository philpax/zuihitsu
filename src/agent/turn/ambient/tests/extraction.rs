//! Tests for the query-extraction concern: stopword filtering, language detection, subphrase building,
//! sentence boundaries, and the query-count cap.

use crate::agent::turn::ambient::query::{MAX_QUERIES, extract_queries};

#[test]
fn extraction_drops_stopwords_and_keeps_content_keywords() {
    // The Stopwords-ISO list is deliberately aggressive: low-signal verbs like "think" go too,
    // leaving the words that actually discriminate ("bonsai", "migration").
    let queries = extract_queries("What do you think of bonsai? It handles schema migration.");
    assert!(queries.contains(&"bonsai".to_owned()));
    assert!(queries.contains(&"migration".to_owned()));
    assert!(!queries.iter().any(|q| q == "what"
        || q == "do"
        || q == "you"
        || q == "of"
        || q == "think"));
}

#[test]
fn extraction_uses_the_detected_language_s_stopwords() {
    // A reliably German message drops German function words and keeps the content nouns.
    let queries = extract_queries(
        "Ich habe gestern lange mit dem Team über die Datenbankmigration gesprochen, \
         und wir sollten das Werkzeug bald aktualisieren.",
    );
    assert!(queries.contains(&"datenbankmigration".to_owned()));
    assert!(queries.contains(&"werkzeug".to_owned()));
    assert!(!queries.iter().any(|q| q == "ich"
        || q == "und"
        || q == "das"
        || q == "mit"
        || q == "die"));
}

#[test]
fn an_unreliable_detection_falls_back_to_english_stopwords() {
    // Too short and ambiguous to detect reliably: the English list applies, so English function
    // words are still dropped rather than fanned out as queries.
    let queries = extract_queries("the bonsai");
    assert!(queries.contains(&"bonsai".to_owned()));
    assert!(!queries.iter().any(|q| q == "the"));
}

#[test]
fn extraction_builds_bigrams_and_trigrams_within_a_sentence() {
    let queries = extract_queries("The migration ships Friday.");
    // Content words: migration, ships, friday.
    assert!(queries.contains(&"migration ships friday".to_owned()));
    assert!(queries.contains(&"migration ships".to_owned()));
    assert!(queries.contains(&"ships friday".to_owned()));
    // Trigrams rank ahead of the bare keywords.
    let tri = queries
        .iter()
        .position(|q| q == "migration ships friday")
        .unwrap();
    let key = queries.iter().position(|q| q == "migration").unwrap();
    assert!(
        tri < key,
        "the trigram claims the budget before the keyword"
    );
}

#[test]
fn extraction_does_not_bridge_a_sentence_boundary() {
    let queries = extract_queries("Deploy bonsai. Erin waits.");
    // "bonsai" and "erin" are in different sentences, so no subphrase joins them.
    assert!(
        !queries
            .iter()
            .any(|q| q.contains("bonsai") && q.contains("erin"))
    );
}

#[test]
fn extraction_caps_the_query_count() {
    let long = (0..200)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(extract_queries(&long).len() <= MAX_QUERIES);
}

#[test]
fn extraction_handles_punctuation_and_empty_text() {
    assert!(extract_queries("").is_empty());
    assert!(extract_queries("!!! ... ??? ;;;").is_empty());
    let queries = extract_queries("Bonsai's schema-migration tool!");
    assert!(queries.iter().any(|q| q.contains("bonsai")));
    assert!(queries.iter().any(|q| q.contains("schema-migration")));
}
