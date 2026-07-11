//! Near-match ranking for a name or tag collision. When a `create` collides with an existing handle,
//! the teachable error lists the neighbouring handles so the agent picks a distinguishing name
//! (`person/dave-chen` versus `person/dave-patel`) rather than colliding again or minting a
//! near-duplicate. The ranking is a pure function of the attempted key and the candidate keys, so it
//! is deterministic under replay.

/// The most suggestions a collision error lists — enough to disambiguate a small cluster of
/// near-duplicates without burying the base message.
pub(super) const MAX_SUGGESTIONS: usize = 5;

/// Rank `candidates` by similarity of their comparison key to `attempted`, returning up to
/// [`MAX_SUGGESTIONS`] of the most similar, closest first. Each candidate is `(key, item)`: the `key`
/// is the string compared (a handle's subject within its namespace, or a tag's name), and `item` is
/// what the caller wants back (the full handle or tag). The exact collider — a key equal to
/// `attempted` — is dropped, since the base error already names it; only the *near* matches remain.
///
/// A candidate is relevant when it shares a leading run with `attempted` (the same stem, how a
/// distinguishing suffix reads) or is a small edit away while still sharing a leading character (a
/// typo). Relevant candidates are ordered by shared-prefix length, then edit distance, then the key
/// itself — the last a total tie-break, so the list is deterministic regardless of the candidates'
/// input order.
pub(super) fn most_similar<T>(attempted: &str, candidates: Vec<(String, T)>) -> Vec<T> {
    let attempted_chars = attempted.chars().count();
    let min_prefix = attempted_chars.min(MIN_SHARED_PREFIX);
    let mut ranked: Vec<(usize, usize, String, T)> = candidates
        .into_iter()
        .filter_map(|(key, item)| {
            if key.eq_ignore_ascii_case(attempted) {
                return None;
            }
            let shared = shared_prefix_len(attempted, &key);
            // When the stem gate fails, only the typo gate can admit the candidate, and two cheap
            // necessary conditions for it — a shared leading character, and lengths within
            // MAX_EDIT_DISTANCE (the distance is at least the length difference) — are checked
            // before the quadratic distance program, so an irrelevant candidate costs no DP.
            if shared < min_prefix
                && (shared == 0
                    || attempted_chars.abs_diff(key.chars().count()) > MAX_EDIT_DISTANCE)
            {
                return None;
            }
            let distance = edit_distance(attempted, &key);
            // A shared stem (the same beginning, how a distinguishing suffix reads) or a near-typo that
            // still shares a leading character — the second gate keeps a same-length swap like
            // `gadget`/`widget` from reading as a near-match to an unrelated word.
            let relevant = shared >= min_prefix || (distance <= MAX_EDIT_DISTANCE && shared >= 1);
            relevant.then_some((shared, distance, key, item))
        })
        .collect();
    // Most shared prefix first, then closest by edit distance, then the key as a total order.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    ranked
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, _, item)| item)
        .collect()
}

/// The shortest shared leading run that still marks a candidate as a near-match — three characters,
/// so a distinguishing suffix on a real stem (`dave` → `dave-chen`) reads as related while an
/// unrelated handle does not. Clamped to the attempted key's own length for a very short stem.
const MIN_SHARED_PREFIX: usize = 3;

/// The largest edit distance that still marks a candidate as a near-match, independent of any shared
/// prefix — so a typo or a plural (`meeting` ⇄ `meetings`) is caught even when the divergence is not
/// at the tail.
const MAX_EDIT_DISTANCE: usize = 2;

/// The number of leading characters `a` and `b` share, compared case-insensitively so a stray capital
/// does not read as a divergence.
fn shared_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
        .count()
}

/// The Levenshtein edit distance between `a` and `b` (insertions, deletions, and substitutions),
/// over Unicode characters. A two-row dynamic program, so it allocates only the width of the shorter
/// operand.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::{edit_distance, most_similar};

    /// Rank a set of bare keys, pairing each with itself as the returned item.
    fn rank(attempted: &str, keys: &[&str]) -> Vec<String> {
        let candidates = keys
            .iter()
            .map(|key| (key.to_string(), key.to_string()))
            .collect();
        most_similar(attempted, candidates)
    }

    #[test]
    fn drops_the_exact_collider() {
        // The base error already names the exact collision, so it is never suggested back.
        let suggestions = rank("widget", &["widget", "widget-blue"]);
        assert_eq!(suggestions, vec!["widget-blue"]);
    }

    #[test]
    fn ranks_shared_stems_by_edit_distance_and_drops_the_unrelated() {
        // Both widgets share the whole `widget` stem, so they rank by edit distance (the shorter
        // suffix first); `gadget` is a same-length swap that shares no leading character and is dropped.
        let suggestions = rank("widget", &["gadget", "widget-blue", "widget-red"]);
        assert_eq!(suggestions, vec!["widget-red", "widget-blue"]);
    }

    #[test]
    fn catches_a_near_typo_that_shares_a_leading_character() {
        // Too short a shared prefix to pass the stem gate, but a one-character divergence that still
        // shares a leading character is a near-match; `calendar` shares nothing and is dropped.
        let suggestions = rank("meeting", &["meating", "calendar"]);
        assert_eq!(suggestions, vec!["meating"]);
    }

    #[test]
    fn excludes_the_unrelated() {
        // A key sharing neither a stem nor a near-typo is not surfaced at all.
        assert!(rank("widget", &["sprocket"]).is_empty());
    }

    #[test]
    fn caps_the_list_and_orders_deterministically() {
        let keys = [
            "widget-6", "widget-4", "widget-2", "widget-5", "widget-1", "widget-3",
        ];
        let suggestions = rank("widget", &keys);
        // Every candidate is a near-match, so the cap holds and the tie-break is the key itself.
        assert_eq!(
            suggestions,
            vec!["widget-1", "widget-2", "widget-3", "widget-4", "widget-5"]
        );
    }

    #[test]
    fn short_stem_requires_a_full_shared_prefix() {
        // A two-character attempt clamps the shared-prefix floor to its own length, so only keys that
        // begin with it (or are an edit away) qualify — `ab` matches `abbey`, not `xyzzy`.
        let suggestions = rank("ab", &["abbey", "xyzzy"]);
        assert_eq!(suggestions, vec!["abbey"]);
    }

    #[test]
    fn edit_distance_counts_single_operations() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", "abc"), 0);
    }

    #[test]
    fn a_long_suffix_on_a_shared_stem_is_kept_despite_the_length_gap() {
        // The length pre-filter applies only when the stem gate fails — a candidate sharing the
        // stem is admitted whatever its length.
        let suggestions = rank("widget", &["widget-with-a-very-long-suffix"]);
        assert_eq!(suggestions, vec!["widget-with-a-very-long-suffix"]);
    }

    #[test]
    fn a_short_shared_prefix_with_a_large_length_gap_is_excluded() {
        // Shares one leading character, but the length gap alone puts the edit distance beyond the
        // typo gate — the candidate is dropped without running the distance program.
        assert!(rank("meet", &["mortgage-repayments"]).is_empty());
    }
}
