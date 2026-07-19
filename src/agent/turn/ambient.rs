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
    ids::{MemoryId, MemoryName, TurnId},
    message_refs,
    settings::AmbientSettings,
};

/// The most queries a single message fans out — a bound so a pathological message stays cheap. The
/// budget is filled longest-subphrase-first, so the most specific phrases claim it.
const MAX_QUERIES: usize = 48;

/// The most turn tokens the hint names — a message citing many moments points at the first few, so the
/// lead-in stays terse rather than reprinting a wall of resolvers.
const MAX_TURN_TOKENS: usize = 3;

/// The most memory tokens the hint resolves — a message citing many `[mem:<id>]` references names the
/// first few, so the lead-in stays terse rather than reprinting a wall of handles.
const MAX_MEM_TOKENS: usize = 3;

/// The most URLs the hint names — a message carrying many links points at the first few, so the
/// lead-in stays terse rather than reprinting a wall of fetch pointers.
const MAX_URLS: usize = 2;

/// The most lexical hits fetched per query — small, since the pass unions and re-ranks across queries.
const PER_QUERY_LIMIT: usize = 3;

/// The result of a firing ambient pass: the rendered hint the turn injects as a system message, and
/// the structured hits recorded on the [`crate::event::EventPayload::AmbientRecallSurfaced`] event so
/// the console can show what the model was pointed at.
pub(crate) struct AmbientHint {
    pub message: String,
    pub hits: Vec<AmbientHit>,
}

/// Run the ambient recall pass over `inbound`, excluding any memory in `exclude` — the ids the frozen
/// brief already surfaces (present set, working set, current room, and self), so the hint never
/// restates what the prompt already carries. Exclusion and deduplication both resolve to the `same_as`
/// class primary, so a merged identity surfaces once, under its primary, and excluding one member
/// excludes the whole class. `transcripts_enabled` reflects the instance's
/// `transcripts` feature: when it is on, a `[turn:<id>]` token in the message leads the hint with an
/// explicit `convo.turn` pointer, so the reference is never treated as inert (the resolver is
/// feature-gated, so a token line would be cruel where the feature is off). `browsing_enabled`
/// reflects the instance's `browsing` feature: when it is on, an http(s) URL in the message adds a line
/// pointing at reading it with `web.markdown`, so a shared link is never treated as inert (the tool is
/// feature-gated, so a URL line would be cruel where the feature is off). A `[mem:<id>]` reference is
/// always resolved — memory is never feature-gated — and leads the hint with a line decoding the token
/// to the handle it points at, so a spliced @mention or a pasted reference is legible. Returns `None`
/// when the pass is disabled, or when no memory reference, turn token, URL, or salient, un-excluded
/// lexical hit survives.
pub(crate) fn ambient_recall(
    graph: &Graph,
    settings: &AmbientSettings,
    inbound: &str,
    exclude: &HashSet<MemoryId>,
    transcripts_enabled: bool,
    browsing_enabled: bool,
) -> Result<Option<AmbientHint>, GraphError> {
    if !settings.enabled {
        return Ok(None);
    }
    // The turn tokens the message cites, capped, when transcript resolution is on. A message may point
    // at a recorded moment and carry no lexical hit at all, so the pass now fires on tokens alone —
    // meeting the reluctance to call `convo.turn` structurally rather than leaving the pointer inert.
    let tokens: Vec<TurnId> = if transcripts_enabled {
        // A message repeating one token gets one line: dedup (first occurrence wins) before the cap.
        let mut seen = HashSet::new();
        let mut ids: Vec<TurnId> = message_refs::extract_turn_ids(inbound)
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect();
        ids.truncate(MAX_TURN_TOKENS);
        ids
    } else {
        Vec::new()
    };

    // The http(s) URLs the message carries, capped, when browsing is on. A message may share a link and
    // carry no lexical hit at all, so the pass fires on URLs alone — meeting the `web.markdown` pointer
    // structurally rather than leaving a shared link inert.
    let urls: Vec<String> = if browsing_enabled {
        // A message repeating one URL gets one line: dedup (first occurrence wins) before the cap.
        let mut seen = HashSet::new();
        let mut found: Vec<String> = extract_urls(inbound)
            .into_iter()
            .filter(|url| seen.insert(url.clone()))
            .collect();
        found.truncate(MAX_URLS);
        found
    } else {
        Vec::new()
    };

    // The memory references the message cites, resolved to their handles. Memory is never feature-gated,
    // so this always runs: a spliced `[mem:<id>]` — a connector's rendering of a platform @mention, or a
    // pasted reference — is opaque until the hint names what it points at, so the agent operates on the
    // handle natively rather than on the token. Resolution collapses to the `same_as` class primary, so a
    // referenced member of a merged identity names the class. Exclusion does not apply: the token must be
    // decoded whether or not the subject is already in the brief. An id that resolves to no live memory
    // (perhaps from another instance) gets no line — a silent skip.
    let mems: Vec<ResolvedMem> = {
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();
        for id in message_refs::extract_mem_ids(inbound) {
            if !seen.insert(id) {
                continue;
            }
            let primary = graph.class_id(id)?.unwrap_or(id);
            if let Some(memory) = graph.memory_by_id(primary)? {
                resolved.push(ResolvedMem {
                    token: id,
                    name: memory.name,
                });
                if resolved.len() >= MAX_MEM_TOKENS {
                    break;
                }
            }
        }
        resolved
    };

    // The excluded ids resolved to their class primaries, so excluding one member of a merged `same_as`
    // identity (present set or brief) excludes the whole class.
    let excluded: HashSet<MemoryId> = exclude
        .iter()
        .map(|id| Ok(graph.class_id(*id)?.unwrap_or(*id)))
        .collect::<Result<_, GraphError>>()?;

    // The best (most negative bm25) score seen for each class across every query that matched it, with
    // the snippet of that best-scoring query — so an identity hit by several queries or across its
    // merged stubs keeps its strongest evidence and surfaces once, under its class primary.
    let queries = extract_queries(inbound);
    let mut best: HashMap<MemoryId, (f32, String)> = HashMap::new();
    for query in &queries {
        for hit in graph.search_lexical(query, PER_QUERY_LIMIT)? {
            let primary = graph.class_id(hit.id)?.unwrap_or(hit.id);
            if excluded.contains(&primary) {
                continue;
            }
            match best.get(&primary) {
                Some((score, _)) if *score <= hit.score => {}
                _ => {
                    best.insert(primary, (hit.score, hit.snippet));
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
    // Fire when there is anything to say: a surviving lexical hit, a cited turn token, a shared URL, a
    // resolved memory reference, or any combination. A hint carrying only leading lines (tokens, URLs, or
    // mem references) has no `hits`, which the recorded event and its replay handle unchanged.
    if resolved.is_empty() && tokens.is_empty() && urls.is_empty() && mems.is_empty() {
        return Ok(None);
    }

    Ok(Some(AmbientHint {
        message: render(&mems, &tokens, &urls, &resolved),
        hits,
    }))
}

/// A surviving hit resolved to what the hint renders: the memory's handle and the FTS snippet of its
/// strongest match.
struct ResolvedHit {
    name: MemoryName,
    snippet: String,
}

/// A cited memory reference resolved to what the hint renders: the token's id (as it appears in the
/// message) and the handle it points at, so the agent can decode `[mem:<id>]` to a memory it operates on
/// by handle.
struct ResolvedMem {
    token: MemoryId,
    name: MemoryName,
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

/// Render the hint the turn injects: first one line per resolved memory reference, decoding `[mem:<id>]`
/// to the handle it points at (so a spliced @mention or pasted reference is never opaque), then one line
/// per cited turn token pointing at its `convo.turn` resolver (so an explicit reference is never inert),
/// then one line per shared URL pointing at reading it with `web.markdown` (so a shared link is never
/// inert), then — when lexical hits survive — the "possibly relevant" block, one line per hit naming the
/// handle and its snippet. It sits after the inbound message in the prompt, so it reads as a note about
/// that message. At least one of `mems`, `tokens`, `urls`, or `hits` is non-empty (the caller returns
/// `None` otherwise).
fn render(
    mems: &[ResolvedMem],
    tokens: &[TurnId],
    urls: &[String],
    hits: &[ResolvedHit],
) -> String {
    let mut out = String::new();
    for mem in mems {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(
            out,
            "[mem:{}] refers to {} — read it with memory.get(\"{}\") if useful.",
            mem.token.0,
            mem.name.as_str(),
            mem.name.as_str()
        );
    }
    for token in tokens {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(
            out,
            "The message above references a recorded moment — read it with convo.turn(\"{}\").",
            token.0
        );
    }
    for url in urls {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(
            out,
            "The message includes a link — read it with web.markdown(\"{url}\")."
        );
    }
    if !hits.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Possibly relevant to the message above — read with memory.get if useful:");
        for hit in hits {
            let snippet = hit.snippet.trim();
            if snippet.is_empty() {
                let _ = write!(out, "\n- {}", hit.name.as_str());
            } else {
                let _ = write!(out, "\n- {} — \"{snippet}\"", hit.name.as_str());
            }
        }
    }
    out
}

/// Extract the http(s) URLs an inbound message carries, in order of appearance. The scan is minimal and
/// scheme-anchored: from each `http://` or `https://` it takes characters up to the first ASCII
/// whitespace or a closing delimiter that bounds a URL embedded in prose, markdown, or brackets, then
/// strips trailing sentence punctuation. A bare scheme with no host is discarded. A missed exotic form
/// costs nothing — the pointer is a nudge, not a parser. Dedup and the cap happen in the caller.
fn extract_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut search_from = 0;
    while let Some(start) = find_scheme(text, search_from) {
        let rest = &text[start..];
        let scheme_len = if rest.starts_with("https://") { 8 } else { 7 };
        // Take the span from the scheme up to the first bounding character.
        let span_end = rest[scheme_len..]
            .find(|c: char| c.is_ascii_whitespace() || is_url_boundary(c))
            .map(|offset| scheme_len + offset)
            .unwrap_or(rest.len());
        let span = &rest[..span_end];
        let trimmed = span.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '>']);
        // Keep only a URL that carries a host after the scheme.
        if trimmed.len() > scheme_len {
            urls.push(trimmed.to_owned());
        }
        search_from = start + span_end;
    }
    urls
}

/// The byte index at or after `from` where the next `http://` or `https://` scheme begins, or `None`.
/// `str::find` returns a char-boundary index, so the caller may slice `text` at it safely.
fn find_scheme(text: &str, from: usize) -> Option<usize> {
    let haystack = &text[from..];
    match (haystack.find("http://"), haystack.find("https://")) {
        (Some(a), Some(b)) => Some(from + a.min(b)),
        (Some(a), None) => Some(from + a),
        (None, Some(b)) => Some(from + b),
        (None, None) => None,
    }
}

/// The characters that bound a URL embedded in prose, markdown, or brackets — the closing side of a
/// wrapping pair, or a shell/markdown metacharacter that never appears mid-URL in practice.
fn is_url_boundary(c: char) -> bool {
    matches!(
        c,
        '<' | '>' | '"' | '\'' | '`' | ')' | ']' | '}' | '|' | '\\'
    )
}

#[cfg(test)]
mod tests {
    use super::{ResolvedHit, ResolvedMem, ambient_recall, extract_queries, extract_urls, render};
    use crate::{
        event::{Cardinality, EventPayload, LinkPosture, LinkSource, Teller, Visibility},
        graph::Graph,
        ids::{EntryId, MemoryId, MemoryName, Namespace, TurnId},
        mem_ref,
        settings::AmbientSettings,
        store::{MemoryStore, Store},
        time::Timestamp,
        turn_ref,
        vocabulary::RelationName,
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

    /// A person stub named by its full handle, with one public content entry — the shape a merged
    /// `same_as` class is built from.
    fn person(id: MemoryId, name: &str, text: &str) -> Vec<EventPayload> {
        vec![
            EventPayload::memory_created(id, MemoryName::new(name)),
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

    /// Merge `a` and `b` into one `same_as` class (operator-adjudicated), mirroring the graph merge
    /// tests' payload pattern.
    fn same_as(a: MemoryId, b: MemoryId) -> Vec<EventPayload> {
        vec![
            EventPayload::LinkTypeRegistered {
                name: RelationName::SameAs,
                inverse: RelationName::SameAs,
                from_card: Cardinality::Many,
                to_card: Cardinality::Many,
                symmetric: true,
                reflexive: false,
                description: String::new(),
            },
            EventPayload::link_created(
                a,
                b,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
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
            true,
            true,
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
            true,
            true,
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
            true,
            true,
        )
        .unwrap();
        assert!(hint.is_none(), "an excluded memory is dropped");
    }

    /// Two stubs of one identity, merged into one `same_as` class, both matching the kelp survey text.
    fn merged_rowan() -> (Graph, MemoryId, MemoryId) {
        let direct = MemoryId::generate();
        let chat = MemoryId::generate();
        let mut payloads = person(
            direct,
            "person/rowan@direct",
            "Kelp survey planning at the harbour.",
        );
        payloads.extend(person(
            chat,
            "person/rowan@chat",
            "Kelp logistics notes from the night shift.",
        ));
        payloads.extend(same_as(direct, chat));
        (corpus(payloads), direct, chat)
    }

    #[test]
    fn a_merged_class_surfaces_once_under_its_primary() {
        // Both stubs match the inbound text, but the class collapses to one hint line, under its
        // primary, rather than naming the identity twice.
        let (graph, direct, chat) = merged_rowan();
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            "Any news on the kelp survey?",
            &HashSet::new(),
            true,
            true,
        )
        .unwrap()
        .expect("the merged class surfaces a hint");
        assert_eq!(hint.hits.len(), 1, "the class surfaces as one hit");
        assert_eq!(
            hint.hits[0].memory,
            direct.min(chat),
            "the hit is the class primary"
        );
        let lines = hint.message.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(lines, 1, "one hint line for the class: {}", hint.message);
    }

    #[test]
    fn excluding_one_member_suppresses_the_whole_class() {
        // Excluding a non-primary member (say the frozen brief surfaced it) resolves to the class
        // primary, so the whole identity is suppressed rather than surfacing under its other stub.
        let (graph, direct, chat) = merged_rowan();
        let mut exclude = HashSet::new();
        exclude.insert(direct.max(chat));
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            "Any news on the kelp survey?",
            &exclude,
            true,
            true,
        )
        .unwrap();
        assert!(
            hint.is_none(),
            "excluding one member excludes the whole class"
        );
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
            true,
            true,
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
        let hint = ambient_recall(
            &graph,
            &capped,
            "database migration tool",
            &HashSet::new(),
            true,
            true,
        )
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
                &HashSet::new(),
                true,
                true,
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
        let out = render(&[], &[], &[], &hits);
        let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 2, "one line per hit");
        assert!(lines[0].contains("topic/bonsai") && lines[0].contains("schema-migration"));
        // An empty snippet renders the handle alone, with no dangling quotes.
        assert_eq!(lines[1], "- topic/driftwood");
    }

    #[test]
    fn a_turn_token_leads_the_hint_and_fires_without_a_lexical_hit() {
        // A message that cites a recorded moment but matches nothing lexically still surfaces a hint:
        // the token line leads, pointing at convo.turn, so the reference is never inert.
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built.",
        ));
        let turn = TurnId::generate();
        let message = format!(
            "Can you dig up what we said in {}?",
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
        .expect("the token fires the hint even with no lexical hit");
        assert!(hint.hits.is_empty(), "no lexical hit rode along");
        let first = hint.message.lines().next().unwrap();
        assert!(
            first.contains(&format!("convo.turn(\"{}\")", turn.0)),
            "the hint leads with the token's resolver: {first}"
        );
    }

    #[test]
    fn a_turn_token_leads_before_the_lexical_block() {
        // With both a token and a salient lexical hit, the token line leads and the "possibly relevant"
        // block follows.
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built; it versions and applies database migrations.",
        ));
        let turn = TurnId::generate();
        let message = format!(
            "What do you think of bonsai, given {}?",
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
        .expect("both a token and a lexical hit surface");
        assert_eq!(hint.hits.len(), 1);
        let token_line = hint
            .message
            .lines()
            .position(|l| l.contains("convo.turn"))
            .expect("a token line");
        let lexical_line = hint
            .message
            .lines()
            .position(|l| l.contains("Possibly relevant"))
            .expect("the lexical block");
        assert!(
            token_line < lexical_line,
            "the token line leads: {}",
            hint.message
        );
    }

    #[test]
    fn a_turn_token_is_silent_when_transcripts_are_off() {
        // The convo.turn resolver is transcripts-gated, so with the feature off a token yields no line —
        // and, with no lexical hit either, no hint at all (nudging at a nil call would be cruel).
        let bonsai = MemoryId::generate();
        let graph = corpus(topic(
            bonsai,
            "bonsai",
            "A schema-migration tool Erin built.",
        ));
        let turn = TurnId::generate();
        let message = format!(
            "Can you dig up what we said in {}?",
            turn_ref::construct(turn)
        );
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            &message,
            &HashSet::new(),
            false,
            true,
        )
        .unwrap();
        assert!(
            hint.is_none(),
            "no token line and no lexical hit means no hint"
        );
    }

    #[test]
    fn the_token_lead_caps_at_the_first_few() {
        // A message citing many moments names only the first MAX_TURN_TOKENS, so the lead-in stays terse.
        let turns: Vec<TurnId> = (0..5).map(|_| TurnId::generate()).collect();
        let mut message = String::from("Compare these:");
        for turn in &turns {
            message.push(' ');
            message.push_str(&turn_ref::construct(*turn));
        }
        let graph = corpus(Vec::new());
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            &message,
            &HashSet::new(),
            true,
            true,
        )
        .unwrap()
        .expect("the tokens fire the hint");
        let token_lines = hint
            .message
            .lines()
            .filter(|l| l.contains("convo.turn"))
            .count();
        assert_eq!(token_lines, super::MAX_TURN_TOKENS, "the lead-in is capped");
    }

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
        assert_eq!(url_lines, super::MAX_URLS, "the URL lead-in is capped");
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
    fn a_mem_reference_fires_the_hint_without_a_lexical_hit() {
        // A message carrying a spliced `[mem:<id>]` but matching nothing lexically still surfaces a
        // hint: the mem line decodes the token to its handle, so the reference is never opaque.
        let rowan = MemoryId::generate();
        let graph = corpus(person(rowan, "person/rowan@chat", "Runs the boat crew."));
        let message = format!("is {} reachable right now?", mem_ref::construct(rowan));
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            &message,
            &HashSet::new(),
            true,
            true,
        )
        .unwrap()
        .expect("the mem token fires the hint even with no lexical hit");
        assert!(hint.hits.is_empty(), "no lexical hit rode along");
        let first = hint.message.lines().next().unwrap();
        assert!(
            first.contains(&format!("[mem:{}] refers to person/rowan@chat", rowan.0)),
            "the hint leads with the decoded token: {first}"
        );
    }

    #[test]
    fn a_referenced_merged_member_names_the_class_primary() {
        // Referencing the non-primary member's token collapses to the class primary's handle, so a
        // merged identity reads under one handle — but the line still names the token as the message
        // wrote it.
        let (graph, direct, chat) = merged_rowan();
        let primary = direct.min(chat);
        let non_primary = direct.max(chat);
        let message = format!("has {} been around?", mem_ref::construct(non_primary));
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            &message,
            &HashSet::new(),
            true,
            true,
        )
        .unwrap()
        .expect("the mem token fires the hint");
        let primary_name = graph.memory_by_id(primary).unwrap().unwrap().name;
        let mem_line = hint
            .message
            .lines()
            .find(|l| l.contains("refers to"))
            .expect("a mem line");
        assert!(
            mem_line.contains(primary_name.as_str()),
            "the line names the class primary: {mem_line}"
        );
        assert!(
            mem_line.contains(&format!("[mem:{}]", non_primary.0)),
            "the line names the token as written: {mem_line}"
        );
    }

    #[test]
    fn an_unresolvable_mem_reference_is_silent() {
        // A token for a memory that does not exist here (perhaps from another instance) resolves to no
        // handle, so it gets no line — and with no lexical hit either, no hint at all.
        let graph = corpus(Vec::new());
        let ghost = MemoryId::generate();
        let message = format!("what about {}?", mem_ref::construct(ghost));
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            &message,
            &HashSet::new(),
            true,
            true,
        )
        .unwrap();
        assert!(
            hint.is_none(),
            "an unresolvable mem reference yields no line and no hint"
        );
    }

    #[test]
    fn the_mem_lead_caps_at_the_first_few() {
        // A message citing many mem references names only the first MAX_MEM_TOKENS, so the lead-in
        // stays terse.
        let ids: Vec<MemoryId> = (0..5).map(|_| MemoryId::generate()).collect();
        let mut payloads = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            payloads.extend(person(
                *id,
                &format!("person/user{i}@chat"),
                "a crew member",
            ));
        }
        let graph = corpus(payloads);
        let mut message = String::from("who among these:");
        for id in &ids {
            message.push(' ');
            message.push_str(&mem_ref::construct(*id));
        }
        let hint = ambient_recall(
            &graph,
            &AmbientSettings::default(),
            &message,
            &HashSet::new(),
            true,
            true,
        )
        .unwrap()
        .expect("the mem tokens fire the hint");
        let mem_lines = hint
            .message
            .lines()
            .filter(|l| l.contains("refers to"))
            .count();
        assert_eq!(
            mem_lines,
            super::MAX_MEM_TOKENS,
            "the mem lead-in is capped"
        );
    }

    #[test]
    fn a_mem_line_leads_before_the_turn_and_url_lines() {
        // With a mem reference, a turn token, and a URL, the order is: mem line, then turn line, then
        // URL line.
        let rowan = MemoryId::generate();
        let graph = corpus(person(rowan, "person/rowan@chat", "Runs the boat crew."));
        let turn = TurnId::generate();
        let message = format!(
            "has {} seen {} at https://example.com/x?",
            mem_ref::construct(rowan),
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
        .expect("a mem ref, a turn token, and a URL surface");
        let mem_line = hint
            .message
            .lines()
            .position(|l| l.contains("refers to"))
            .expect("a mem line");
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
        assert!(
            mem_line < token_line && token_line < url_line,
            "mem, then turn, then URL: {}",
            hint.message
        );
    }
}
