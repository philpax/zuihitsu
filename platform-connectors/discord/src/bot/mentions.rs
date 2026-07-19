//! Content rewriting: turning raw Discord mentions into canonical memory tokens, while leaving
//! mentions inside code spans as literal samples.

use std::collections::HashMap;

use serenity::all::UserId;

use zuihitsu_core::{ids::MemoryId, mem_ref};

/// Rewrite each raw Discord mention (`<@id>` and the nickname form `<@!id>`) of a projected user as the
/// canonical `[mem:<id>]` memory token, so the agent reads a stable reference rather than an opaque
/// platform mention and the console renders a link. A user absent from `memory_ids` keeps its raw
/// mention: the bot's own mention (addressing, not reference) is never in the map, and a mention whose
/// projection failed degrades to its raw form. Parsing `<@…>` is the connector reading its own platform's
/// syntax.
///
/// A mention inside a Discord code span — a backtick-delimited inline run or a triple-backtick fenced
/// block — is left raw, since there it is a literal code sample, not a reference. An unclosed backtick
/// run is not a span at all (Discord renders the backticks literally), so scanning resumes after it and
/// any mention in the prose beyond still splices.
pub(super) fn splice_mentions(text: &str, memory_ids: &HashMap<UserId, MemoryId>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = backtick_run(bytes, i);
            if let Some(end) = closing_run(bytes, i + run, run) {
                // A closed code span: copy it verbatim, backticks and content alike, so a mention
                // inside stays a literal code sample.
                out.push_str(&text[i..end]);
                i = end;
            } else {
                // An unclosed run: the backticks are literal, not a span. Emit them and resume normal
                // scanning, so a mention in the prose beyond still splices.
                out.push_str(&text[i..i + run]);
                i += run;
            }
            continue;
        }
        if let Some((user_id, len)) = mention_at(text, i)
            && let Some(memory_id) = memory_ids.get(&user_id)
        {
            out.push_str(&mem_ref::construct(*memory_id));
            i += len;
            continue;
        }
        // Not a mention we splice: copy one whole character so the scan never slices mid-character.
        let ch = text[i..].chars().next().expect("i is a char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The length in bytes of the run of backticks starting at `i` (where `bytes[i]` is a backtick). Backticks
/// are ASCII, so the run boundary is always a character boundary.
fn backtick_run(bytes: &[u8], i: usize) -> usize {
    let mut n = 0;
    while i + n < bytes.len() && bytes[i + n] == b'`' {
        n += 1;
    }
    n
}

/// The byte offset just past the backtick run that closes a code span of length `run`, searching from
/// `from`, or `None` when the span never closes. Discord closes a span on the next run of exactly the
/// opening length: a run of a different length is skipped whole, so its backticks are never miscounted as
/// a partial close.
fn closing_run(bytes: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let len = backtick_run(bytes, j);
            if len == run {
                return Some(j + run);
            }
            j += len;
        } else {
            j += 1;
        }
    }
    None
}

/// The Discord mention starting at byte `i`, if `text[i..]` opens `<@id>` or the nickname form `<@!id>`
/// with a numeric id, and the mention's byte length. `None` when no mention opens there.
fn mention_at(text: &str, i: usize) -> Option<(UserId, usize)> {
    let rest = text.get(i..)?.strip_prefix("<@")?;
    // The nickname form carries a leading `!` before the id; both forms name the same user.
    let after_bang = rest.strip_prefix('!').unwrap_or(rest);
    let bang_len = rest.len() - after_bang.len();
    let digits: String = after_bang
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() || !after_bang[digits.len()..].starts_with('>') {
        return None;
    }
    let id = digits.parse::<u64>().ok()?;
    // "<@" + optional "!" + digits + ">".
    let len = "<@".len() + bang_len + digits.len() + '>'.len_utf8();
    Some((UserId::new(id), len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_id(bits: u128) -> MemoryId {
        MemoryId(ulid::Ulid::from(bits))
    }

    #[test]
    fn splice_rewrites_both_mention_forms_of_a_projected_user() {
        let dave = UserId::new(123);
        let mem = memory_id(1);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);

        // Both the plain and the nickname form of the same user splice to the same token.
        assert_eq!(
            splice_mentions("hey <@123> around?", &map),
            format!("hey {token} around?")
        );
        assert_eq!(
            splice_mentions("hey <@!123> around?", &map),
            format!("hey {token} around?")
        );
    }

    #[test]
    fn splice_leaves_an_unprojected_mention_raw() {
        // The bot's own mention (and any user whose projection failed) is absent from the map, so its
        // raw form is preserved verbatim — addressing, not a reference.
        let map: HashMap<UserId, MemoryId> = HashMap::new();
        assert_eq!(splice_mentions("<@999> hello", &map), "<@999> hello");
        assert_eq!(splice_mentions("<@!999> hi", &map), "<@!999> hi");
    }

    #[test]
    fn splice_rewrites_only_projected_users_among_several_mentions() {
        let dave = UserId::new(123);
        let mem = memory_id(2);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);
        // Dave splices; Erin (unprojected) and the surrounding prose stay exactly as written.
        assert_eq!(
            splice_mentions("cc <@123> and <@456> please", &map),
            format!("cc {token} and <@456> please")
        );
    }

    #[test]
    fn splice_preserves_non_mention_and_multibyte_text() {
        let map: HashMap<UserId, MemoryId> = HashMap::new();
        // A lone `<`, an email-ish `@`, and multibyte prose must scan without panicking or corruption.
        for text in [
            "plain text",
            "a < b and c@d",
            "さっき <@ not a mention",
            "emoji 🎉 done",
        ] {
            assert_eq!(splice_mentions(text, &map), text);
        }
    }

    #[test]
    fn splice_leaves_a_mention_in_inline_code_raw() {
        let dave = UserId::new(123);
        let map = HashMap::from([(dave, memory_id(3))]);
        // A mention inside a backtick-delimited inline run is a literal code sample, not a reference.
        assert_eq!(
            splice_mentions("use `<@123>` to ping", &map),
            "use `<@123>` to ping"
        );
        // A double-backtick run (used so the content may itself contain a single backtick) closes only
        // on a matching double run, leaving the mention within it raw.
        assert_eq!(
            splice_mentions("run ``<@123>`` now", &map),
            "run ``<@123>`` now"
        );
    }

    #[test]
    fn splice_leaves_a_mention_in_a_fenced_block_raw() {
        let dave = UserId::new(123);
        let map = HashMap::from([(dave, memory_id(4))]);
        // A triple-backtick fenced block copies through untouched, mention and all.
        assert_eq!(
            splice_mentions("```\nping <@123> here\n```", &map),
            "```\nping <@123> here\n```"
        );
    }

    #[test]
    fn splice_rewrites_prose_but_not_code_in_a_mixed_message() {
        let dave = UserId::new(123);
        let mem = memory_id(5);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);
        // The prose mention splices; the identical mention inside the inline code stays a raw sample.
        assert_eq!(
            splice_mentions("cc <@123> — sample `<@123>`", &map),
            format!("cc {token} — sample `<@123>`")
        );
    }

    #[test]
    fn splice_treats_an_unclosed_backtick_run_as_literal_and_splices_past_it() {
        let dave = UserId::new(123);
        let mem = memory_id(6);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);
        // Discord renders an unclosed backtick literally, so it opens no span: the backtick is emitted
        // as-is and the mention beyond it still splices.
        assert_eq!(
            splice_mentions("oops `<@123> unclosed", &map),
            format!("oops `{token} unclosed")
        );
    }

    #[test]
    fn mention_at_rejects_malformed_forms() {
        // No id, a non-numeric id, and an unterminated mention are not mentions.
        assert_eq!(mention_at("<@>", 0), None);
        assert_eq!(mention_at("<@abc>", 0), None);
        assert_eq!(mention_at("<@123", 0), None);
        // A well-formed mention reports the right byte length (plain and nickname forms).
        assert_eq!(mention_at("<@123>", 0).map(|(_, len)| len), Some(6));
        assert_eq!(mention_at("<@!123>", 0).map(|(_, len)| len), Some(7));
    }
}
