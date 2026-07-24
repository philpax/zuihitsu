//! Search-query write guards and fuzzy handle matching: the hidden query field a `memory.search` hit
//! carries, the per-handle and per-block taint guards that refuse a content write to a hit whose query
//! does not name it, and the diacritic-folding tokeniser both guards, and `mem:find_entry`, compare on.

use mlua::Table;

use std::collections::HashSet;

use crate::ids::MemoryId;

use crate::agent::lua::{
    error::{SearchWriteError, TaintedWriteError},
    runtime::{BlockApi, route_error},
};

/// The hidden field a `memory.search` result carries: the query it was minted from. Written with
/// `raw_set` so it neither renders (the metatable's `__tostring` never reads it) nor trips the
/// read-only `__newindex` guard, and read back by [`guard_search_write`] to verify a write's target.
pub(crate) const SEARCH_QUERY_FIELD: &str = "search_query";

/// The fuzzy-write guard: refuse a content write (or a `links.create` endpoint) that goes through a
/// `memory.search` hit whose query does not name the handle it landed on. A hit carries its query in
/// [`SEARCH_QUERY_FIELD`]; a handle from `memory.get`/`create`/`get_or_create`/`list` carries none and
/// is never gated (its target is a literal name the agent read, not a fuzzy match). When the
/// provenance is present, some whitespace- or punctuation-delimited token of the query must equal the
/// handle's name segment — the part after `namespace/`, or, for a multi-part segment like
/// `dave_chen`, one of its underscore parts or the parts joined. Exact token equality only: a stem
/// ("dav") never proves identity ("david"), so it does not pass. The traced failure — searching
/// "Davina", taking the person/david hit as her through the `if #hits == 0 then create else hits[1]`
/// idiom, and landing her role on him — is refused with a teachable error before it commits.
pub(crate) fn guard_search_write(handle: &Table) -> mlua::Result<()> {
    let Some(query) = handle.raw_get::<Option<String>>(SEARCH_QUERY_FIELD)? else {
        return Ok(());
    };
    // A hit always carries its `name` beside the query; without it there is nothing to verify against,
    // so treat the (unreachable) absence as ungated rather than raising a spurious refusal.
    let Some(name) = handle.raw_get::<Option<String>>("name")? else {
        return Ok(());
    };
    if query_names_handle(&query, &name) {
        return Ok(());
    }
    let segment = name_segment(&name);
    let namespace = &name[..name.len() - segment.len()];
    let query_token = query_tokens(&query).next().unwrap_or_default();
    let stem = common_prefix(&query_token, &fold_lower(segment));
    let list_arg = if stem.is_empty() {
        namespace.to_owned()
    } else {
        format!("{namespace}{stem}")
    };
    let create_handle = format!("{namespace}{query_token}");
    Err(SearchWriteError {
        query,
        name,
        list_arg,
        create_handle,
    }
    .into())
}

/// The block-scoped taint guard: refuse a content write (or a `links.create` endpoint) whose target
/// memory a `memory.search` this block surfaced without the query naming it. Where [`guard_search_write`]
/// gates the *handle* a write came through — the search hit still carrying its query — this gates the
/// *target name*, however the write reached it: the launder is composing one block that searches, then
/// writes to the mismatched hit through a provenance-free `memory.get(hits[1].name)` handle. Because the
/// whole if/else is written before the search runs, an in-block branch on the result is a guess the
/// model never got to weigh; the block boundary is the only place a judgement can happen. So the write
/// is refused, the retry — a fresh block composed after seeing the error — carries an empty taint map
/// and writes through. That cross-block asymmetry is the point: taint dies with the block.
///
/// The accepted cost: a legitimate same-block write to a memory a mismatched search also surfaced is
/// refused once here and succeeds on the retry block. Cheap — the map is empty on the overwhelmingly
/// common no-mismatch block, so the name resolution only runs when a search already misfired this block.
pub(crate) fn guard_search_taint(api: &BlockApi, id: MemoryId) -> mlua::Result<()> {
    if api.search_taint.lock().is_empty() {
        return Ok(());
    }
    // Resolve the target's current name the way the write sites do (honoring this block's pending
    // creates); an id that resolves to no memory cannot be tainted.
    let Some(name) = api
        .block
        .lock()
        .handle_field(id, "name")
        .map_err(|error| route_error(error, &mut api.infra.lock()))?
    else {
        return Ok(());
    };
    // Take the query out from under the lock before touching the block again, so the two mutexes are
    // never held at once.
    let Some(query) = api.search_taint.lock().get(&name).cloned() else {
        return Ok(());
    };
    let segment = name_segment(&name);
    let namespace = &name[..name.len() - segment.len()];
    let query_token = query_tokens(&query).next().unwrap_or_default();
    let create_handle = format!("{namespace}{query_token}");
    Err(TaintedWriteError {
        query,
        name,
        create_handle,
    }
    .into())
}

/// Whether some normalized token of `query` names `handle_name` — the exact-token match the
/// fuzzy-write guard turns on. Tokens compare after lowercasing and folding diacritics, the way the
/// FTS index folds them: a search for "Malmö" that surfaces `topic/malmo` (the index folded the ö)
/// matches, so the write is not falsely refused. The handle's name segment (the part after
/// `namespace/`) yields the name tokens: its alphanumeric runs (so `dave_chen` gives `dave` and
/// `chen`) plus the whole segment with separators stripped (`davechen`). A query token equals one of
/// those or it does not; a prefix never counts.
pub(crate) fn query_names_handle(query: &str, handle_name: &str) -> bool {
    let names = name_tokens(name_segment(handle_name));
    query_tokens(query).any(|token| names.contains(&token))
}

/// The name segment of a handle — everything after the first `/` (the namespace prefix), or the whole
/// string when it carries no prefix.
fn name_segment(handle_name: &str) -> &str {
    handle_name.split_once('/').map_or(handle_name, |(_, s)| s)
}

/// The tokens a name segment matches against: each alphanumeric run lowercased and diacritic-folded,
/// plus the whole segment with separators removed so a query that runs the parts together (`davechen`)
/// still matches `dave_chen`.
fn name_tokens(segment: &str) -> HashSet<String> {
    let mut tokens: HashSet<String> = query_tokens(segment).collect();
    let joined: String = segment
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(fold_diacritic)
        .collect();
    if !joined.is_empty() {
        tokens.insert(joined);
    }
    tokens
}

/// Split a string into folded alphanumeric tokens, dropping every non-alphanumeric separator —
/// whitespace and punctuation alike — and folding each to the lowercase, diacritic-stripped form the
/// FTS index compares on. `"Marcus Chen"` yields `marcus`, `chen`; `"Malmö"` yields `malmo`.
fn query_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(fold_lower)
}

/// Fold a string to the diacritic-insensitive lowercase form the token match compares on: lowercase,
/// then map each character to its unaccented Latin base. Both sides of the guard fold the same way, so
/// a "Malmö" query and a `topic/malmo` handle meet on `malmo`. Shared with `mem:find_entry`, which
/// folds a needle and each entry's text the same way so a case- or accent-varying phrase still matches.
pub(crate) fn fold_lower(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(fold_diacritic)
        .collect()
}

/// Map a lowercase character to its unaccented Latin base — `ö`→`o`, `é`→`e`, `č`→`c` — over the
/// precomposed Latin-1 Supplement and Latin Extended-A letters that canonically decompose to a base
/// plus a combining mark (an NFD-style fold). This is the folding the FTS index applies, so the guard
/// matches a query against a folded handle exactly as the index indexed it. A character with no such
/// decomposition — a distinct letter like `ø` or `ł`, or any non-Latin script — passes through
/// unchanged.
fn fold_diacritic(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ả' | 'ạ' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ẻ' | 'ẹ' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'ǐ' | 'ỉ' | 'ị' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' | 'ǒ' | 'ỏ' | 'ọ' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => 's',
        'ţ' | 'ť' | 'ț' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ǔ' | 'ủ' | 'ụ' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' | 'ỳ' | 'ỷ' | 'ỵ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

/// The shared leading run of two strings, by character — the stem `memory.list` is suggested with, so
/// a refused "davina"/"david" points the agent at `person/dav` where both spellings show.
fn common_prefix(a: &str, b: &str) -> String {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::query_names_handle;

    #[test]
    fn the_fuzzy_write_guard_folds_diacritics_as_the_index_does() {
        // The FTS index folds diacritics, so a "Malmö" search surfaces `topic/malmo`; the guard folds
        // the same way, so a write through that hit is not falsely refused. Folding runs on both sides.
        assert!(query_names_handle("Malmö", "topic/malmo"));
        assert!(query_names_handle("Malmo", "topic/malmö"));
        assert!(query_names_handle("café society", "topic/cafe"));

        // Folding is diacritic-only — it never collapses two distinct names. The traced Davina/David
        // slip is still refused, and a genuinely different accented name is too.
        assert!(!query_names_handle("Davina", "person/david"));
        assert!(!query_names_handle("Zoë", "person/zara"));
    }
}
