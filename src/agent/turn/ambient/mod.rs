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

mod query;
mod render;
mod url;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::{
    agent::turn::ambient::{
        query::extract_queries,
        render::{ResolvedHit, ResolvedMem, render},
        url::extract_urls,
    },
    event::AmbientHit,
    graph::{Graph, GraphError},
    ids::{MemoryId, TurnId},
    message_refs,
    settings::AmbientSettings,
};

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
