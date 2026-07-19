//! Query extraction for the ambient recall pass: turning an inbound message into the lexical queries it
//! fans out over the graph's FTS index, and the language-aware stopword filtering that keeps those
//! queries to content-bearing terms.

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use parking_lot::Mutex;
use whatlang::Lang;

/// The most queries a single message fans out — a bound so a pathological message stays cheap. The
/// budget is filled longest-subphrase-first, so the most specific phrases claim it.
pub(super) const MAX_QUERIES: usize = 48;

/// Extract the lexical queries an inbound message fans out: the distinct content keywords and the
/// contiguous bigrams and trigrams within each sentence. Sentences split on `.!?;` and newlines, so a
/// subphrase never spans a sentence boundary. Ordered longest-first — trigrams, then bigrams, then
/// keywords — and de-duplicated, so the most specific phrases claim the [`MAX_QUERIES`] budget.
pub(super) fn extract_queries(text: &str) -> Vec<String> {
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
