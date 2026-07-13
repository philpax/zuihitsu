//! The ambient recall pass: a fast, pre-turn lexical retrieval over the inbound message that surfaces
//! memories the frozen brief did not, so the agent recalls what it would not have thought to search for
//! (spec §Conversations and briefs → ambient recall).
//!
//! The pass is pure and cheap: it extracts keywords and short subphrases from the inbound text, fans
//! them out over the graph's lexical (FTS5) index, keeps the salient survivors the brief does not
//! already carry, and renders a terse hint the caller injects as one system message. No model or
//! embedder call is made, so it adds no latency worth budgeting. It reads only the FTS index, which
//! holds public content alone (spec §Visibility → public-only lexical indexing), so a surfaced snippet
//! is public-safe and needs no visibility filter.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::OnceLock,
};

use parking_lot::Mutex;
use whatlang::Lang;

use crate::{
    event::AmbientHit,
    graph::{Graph, GraphError},
    ids::{MemoryId, MemoryName},
    settings::AmbientSettings,
};

/// The most queries a single message fans out — a bound so a pathological message stays cheap. The
/// budget is filled longest-subphrase-first, so the most specific phrases claim it.
const MAX_QUERIES: usize = 48;

/// The most lexical hits fetched per query — small, since the pass unions and re-ranks across queries.
const PER_QUERY_LIMIT: usize = 3;

/// The result of a firing ambient pass: the rendered hint the turn injects as a system message, and
/// the structured hits recorded on the [`crate::event::EventPayload::AmbientRecallSurfaced`] event so
/// the console can show what the model was pointed at.
pub(crate) struct AmbientHint {
    pub message: String,
    pub hits: Vec<AmbientHit>,
}

/// Run the ambient recall pass over `inbound`, excluding any memory whose id is in `exclude` — the ids
/// the frozen brief already surfaces (present set, working set, current room, and self), so the hint
/// never restates what the prompt already carries. Returns `None` when the pass is disabled, the text
/// yields no query, or nothing salient and un-excluded survives.
pub(crate) fn ambient_recall(
    graph: &Graph,
    settings: &AmbientSettings,
    inbound: &str,
    exclude: &HashSet<MemoryId>,
) -> Result<Option<AmbientHint>, GraphError> {
    if !settings.enabled {
        return Ok(None);
    }
    let queries = extract_queries(inbound);
    if queries.is_empty() {
        return Ok(None);
    }

    // The best (most negative bm25) score seen for each memory across every query that matched it, with
    // the snippet of that best-scoring query — so a memory hit by several queries keeps its strongest
    // evidence rather than the last one seen.
    let mut best: HashMap<MemoryId, (f32, String)> = HashMap::new();
    for query in &queries {
        for hit in graph.search_lexical(query, PER_QUERY_LIMIT)? {
            if exclude.contains(&hit.id) {
                continue;
            }
            match best.get(&hit.id) {
                Some((score, _)) if *score <= hit.score => {}
                _ => {
                    best.insert(hit.id, (hit.score, hit.snippet));
                }
            }
        }
    }

    // Salience threshold: a bm25 score is more negative for a stronger match, so a hit survives only
    // when its best score is at or below the ceiling.
    let min_score = settings.min_score as f32;
    let mut candidates: Vec<(MemoryId, f32, String)> = best
        .into_iter()
        .filter(|(_, (score, _))| *score <= min_score)
        .map(|(id, (score, snippet))| (id, score, snippet))
        .collect();
    // Strongest first, breaking ties by id so the order is deterministic under replay.
    candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.0.cmp(&b.0.0)));

    let max_hits = settings.max_hits.max(0) as usize;
    let mut hits = Vec::new();
    let mut resolved = Vec::new();
    for (id, score, snippet) in candidates {
        if hits.len() >= max_hits {
            break;
        }
        // Resolve for the display handle. `memory_by_id` returns only live memories, so a hit whose
        // memory has since been soft-deleted is skipped here.
        let Some(memory) = graph.memory_by_id(id)? else {
            continue;
        };
        hits.push(AmbientHit { memory: id, score });
        resolved.push(ResolvedHit {
            name: memory.name,
            snippet,
        });
    }
    if resolved.is_empty() {
        return Ok(None);
    }

    Ok(Some(AmbientHint {
        message: render(&resolved),
        hits,
    }))
}

/// A surviving hit resolved to what the hint renders: the memory's handle and the FTS snippet of its
/// strongest match.
struct ResolvedHit {
    name: MemoryName,
    snippet: String,
}

/// Extract the lexical queries an inbound message fans out: the distinct content keywords and the
/// contiguous bigrams and trigrams within each sentence. Sentences split on `.!?;` and newlines, so a
/// subphrase never spans a sentence boundary. Ordered longest-first — trigrams, then bigrams, then
/// keywords — and de-duplicated, so the most specific phrases claim the [`MAX_QUERIES`] budget.
fn extract_queries(text: &str) -> Vec<String> {
    let stopwords = stopwords_for(text);
    let mut trigrams = Vec::new();
    let mut bigrams = Vec::new();
    let mut keywords = Vec::new();
    for sentence in text.split(['.', '!', '?', ';', '\n', '\r']) {
        // The sentence's content words in order: lowercased, stripped of surrounding punctuation, with
        // stopwords and single characters dropped. Subphrases are built over these, so a bigram bridges
        // the noise words between two content words rather than pairing a word with a stopword.
        let words: Vec<String> = sentence
            .split_whitespace()
            .filter_map(normalize_word)
            .filter(|word| !stopwords.contains(word.as_str()))
            .collect();
        keywords.extend(words.iter().cloned());
        for pair in words.windows(2) {
            bigrams.push(pair.join(" "));
        }
        for triple in words.windows(3) {
            trigrams.push(triple.join(" "));
        }
    }

    let mut seen = HashSet::new();
    let mut queries = Vec::new();
    for query in trigrams.into_iter().chain(bigrams).chain(keywords) {
        if seen.insert(query.clone()) {
            queries.push(query);
            if queries.len() >= MAX_QUERIES {
                break;
            }
        }
    }
    queries
}

/// Normalize one raw token: strip leading and trailing non-alphanumerics, lowercase, and drop it if
/// fewer than two characters remain (a bare letter or stray punctuation carries no lexical signal).
fn normalize_word(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|c: char| !c.is_alphanumeric());
    if trimmed.chars().count() < 2 {
        return None;
    }
    Some(trimmed.to_lowercase())
}

/// The stopword set for the message's language, detected per message and cached per language for the
/// process's life. Detection is confidence-gated, and a language without a stopword list folds to
/// English before the cache is keyed — so each cache entry is a distinct list, and the fallback fails
/// safe: a stopword the wrong list misses becomes a weak query the salience threshold discards anyway.
fn stopwords_for(text: &str) -> &'static HashSet<String> {
    static CACHE: OnceLock<Mutex<HashMap<Lang, &'static HashSet<String>>>> = OnceLock::new();
    let detected = whatlang::detect(text)
        .filter(|info| info.is_reliable())
        .map(|info| info.lang());
    let (lang, code) = detected
        .and_then(|lang| stopword_code(lang).map(|code| (lang, code)))
        .unwrap_or((Lang::Eng, "en"));
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock();
    cache.entry(lang).or_insert_with(|| {
        // Leaked once per language — the language set is a small closed list, so the total is bounded.
        let list: HashSet<String> = stop_words::get(code)
            .iter()
            .map(|word| word.to_lowercase())
            .collect();
        Box::leak(Box::new(list))
    })
}

/// The ISO 639-1 code `stop_words::get` takes for a detected language, or `None` when no list covers
/// it — the one place the enum leaves the type system. CJK is absent deliberately: the lexical index's
/// tokenizer does not segment those scripts, so query-side handling alone cannot reach their content
/// (see the ambient note in docs/limitations.md).
fn stopword_code(lang: Lang) -> Option<&'static str> {
    match lang {
        Lang::Eng => Some("en"),
        Lang::Deu => Some("de"),
        Lang::Fra => Some("fr"),
        Lang::Spa => Some("es"),
        Lang::Por => Some("pt"),
        Lang::Ita => Some("it"),
        Lang::Nld => Some("nl"),
        Lang::Swe => Some("sv"),
        Lang::Dan => Some("da"),
        Lang::Nob => Some("no"),
        Lang::Fin => Some("fi"),
        Lang::Rus => Some("ru"),
        Lang::Ukr => Some("uk"),
        Lang::Pol => Some("pl"),
        Lang::Ces => Some("cs"),
        Lang::Slk => Some("sk"),
        Lang::Hun => Some("hu"),
        Lang::Ron => Some("ro"),
        Lang::Bul => Some("bg"),
        Lang::Ell => Some("el"),
        Lang::Tur => Some("tr"),
        Lang::Ara => Some("ar"),
        Lang::Heb => Some("he"),
        Lang::Hin => Some("hi"),
        Lang::Ind => Some("id"),
        Lang::Vie => Some("vi"),
        _ => None,
    }
}

/// Render the hint the turn injects: a one-line lead-in, then one line per hit naming the handle and
/// its snippet. It sits after the inbound message in the prompt, so it reads as a note about that
/// message.
fn render(hits: &[ResolvedHit]) -> String {
    let mut out =
        String::from("Possibly relevant to the message above — read with memory.get if useful:");
    for hit in hits {
        let snippet = hit.snippet.trim();
        if snippet.is_empty() {
            let _ = write!(out, "\n- {}", hit.name.as_str());
        } else {
            let _ = write!(out, "\n- {} — \"{snippet}\"", hit.name.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ResolvedHit, ambient_recall, extract_queries, render};
    use crate::{
        event::{EventPayload, Teller, Visibility},
        graph::Graph,
        ids::{EntryId, MemoryId, MemoryName, Namespace},
        settings::AmbientSettings,
        store::{MemoryStore, Store},
        time::Timestamp,
    };
    use std::collections::HashSet;

    /// Build an in-memory graph materialized from `payloads` — the pattern the graph's own search
    /// tests use, so the FTS index the pass reads is populated exactly as production's is.
    fn materialized(payloads: Vec<EventPayload>) -> Graph {
        let mut store = MemoryStore::new();
        store
            .append(
                Timestamp::from_millis(1),
                crate::event::EventSource::Agent,
                payloads,
            )
            .unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        graph
    }

    fn topic(id: MemoryId, name: &str, text: &str) -> Vec<EventPayload> {
        vec![
            EventPayload::memory_created(id, Namespace::Topic.with_name(name)),
            EventPayload::MemoryContentAppended {
                id,
                entry_id: EntryId::generate(),
                asserted_at: Timestamp::from_millis(1),
                occurred_at: None,
                text: text.to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ]
    }

    /// A dozen unrelated memories, so the FTS index carries a realistic corpus. bm25 collapses toward
    /// zero on a one- or two-document index (every term is in every document, so its inverse-document
    /// weight vanishes); the filler restores the score separation between a distinctive match and
    /// common-word noise that a real instance sees.
    fn filler() -> Vec<EventPayload> {
        (0..12)
            .flat_map(|i| {
                topic(
                    MemoryId::generate(),
                    &format!("filler{i}"),
                    &format!("Unrelated note {i} about weather, lunch, and travel plans."),
                )
            })
            .collect()
    }

    /// Materialize `target` memories alongside the filler corpus.
    fn corpus(target: Vec<EventPayload>) -> Graph {
        let mut payloads = target;
        payloads.extend(filler());
        materialized(payloads)
    }

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
        assert!(extract_queries(&long).len() <= super::MAX_QUERIES);
    }

    #[test]
    fn extraction_handles_punctuation_and_empty_text() {
        assert!(extract_queries("").is_empty());
        assert!(extract_queries("!!! ... ??? ;;;").is_empty());
        let queries = extract_queries("Bonsai's schema-migration tool!");
        assert!(queries.iter().any(|q| q.contains("bonsai")));
        assert!(queries.iter().any(|q| q.contains("schema-migration")));
    }

    #[test]
    fn a_salient_content_hit_surfaces() {
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built; it versions and applies database migrations.",
        ));
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            "What do you think of bonsai?",
            &HashSet::new(),
        )
        .unwrap()
        .expect("a salient hit surfaces");
        assert_eq!(hint.hits.len(), 1);
        assert_eq!(hint.hits[0].memory, bonsai);
        assert!(hint.message.contains("topic/bonsai"));
    }

    #[test]
    fn a_message_with_no_salient_term_surfaces_nothing() {
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(bonsai, "bonsai", "A schema-migration tool."));
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            "Thanks, talk soon!",
            &HashSet::new(),
        )
        .unwrap();
        assert!(hint.is_none(), "no query term matches the memory");
    }

    #[test]
    fn a_brief_excluded_memory_is_not_hinted() {
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built.",
        ));
        let mut exclude = HashSet::new();
        exclude.insert(bonsai);
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            "What do you think of bonsai?",
            &exclude,
        )
        .unwrap();
        assert!(hint.is_none(), "an excluded memory is dropped");
    }

    #[test]
    fn the_threshold_filters_weak_matches() {
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built.",
        ));
        // Demanding an unreachably strong match (bm25 is bounded near zero on the weak side, and no
        // real match reaches -1000) filters every hit, so the salient bonsai match is dropped.
        let strict = AmbientSettings {
            min_score: -1_000.0,
            ..AmbientSettings::default()
        };
        let hint = ambient_recall(
            &graph,
            &strict,
            "What do you think of bonsai?",
            &HashSet::new(),
        )
        .unwrap();
        assert!(
            hint.is_none(),
            "no hit is strong enough for the strict ceiling"
        );
    }

    #[test]
    fn the_cap_bounds_the_hits() {
        let mut payloads = Vec::new();
        for i in 0..5 {
            payloads.extend(topic(
                MemoryId::generate(),
                &format!("migration{i}"),
                "A database migration tool for schema migration work.",
            ));
        }
        let graph = corpus(payloads);
        let capped = AmbientSettings {
            max_hits: 2,
            ..AmbientSettings::default()
        };
        let hint = ambient_recall(&graph, &capped, "database migration tool", &HashSet::new())
            .unwrap()
            .expect("several memories match");
        assert_eq!(hint.hits.len(), 2, "the cap bounds the surfaced hits");
    }

    #[test]
    fn disabled_surfaces_nothing() {
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built.",
        ));
        let off = AmbientSettings {
            enabled: false,
            ..AmbientSettings::default()
        };
        assert!(
            ambient_recall(
                &graph,
                &off,
                "What do you think of bonsai?",
                &HashSet::new()
            )
            .unwrap()
            .is_none()
        );
    }

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
        let out = render(&hits);
        let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 2, "one line per hit");
        assert!(lines[0].contains("topic/bonsai") && lines[0].contains("schema-migration"));
        // An empty snippet renders the handle alone, with no dangling quotes.
        assert_eq!(lines[1], "- topic/driftwood");
    }
}
