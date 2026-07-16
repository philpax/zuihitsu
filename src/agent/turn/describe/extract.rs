//! The schema types and lenient parsers for the structured synthesis and arbitration replies.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ids::MemoryName,
    model::extract_json_object,
    time::{self, CivilDate, Direction, Rrule, TemporalRef, Timestamp},
};

/// The `synthesize` argument shape (turn-end description + temporal extraction); doubles as the
/// response-format schema, so the schema sent to the model and the parser can't drift. The
/// contradiction verdict is a separate focused call ([`crate::agent::turn::describe::arbitration::arbitrate`]/[`ExtractedArbitration`]), not a
/// field here.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct SynthesizeArgs {
    /// The memory's description as plain third-person prose — no preamble, headings, or notes.
    pub(super) description: String,
    /// One entry per statement that refers to a real-world time; omit statements with no temporal
    /// reference.
    #[serde(default)]
    pub(super) occurrences: Vec<ExtractedOccurrence>,
}

/// One extracted occurrence: the statement it applies to (1-based, as numbered in the prompt) and
/// the time it refers to.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct ExtractedOccurrence {
    pub(super) entry: usize,
    pub(super) occurred_at: ExtractedTime,
}

/// A conflict the focused [`crate::agent::turn::describe::arbitration::arbitrate`] call found among the numbered statements (spec §Write path →
/// arbitration): which statements collide, which the model credits, and a one-line reconciling note. It
/// doubles as that call's response-format schema. Statement numbers are 1-based, the same numbering
/// [`ExtractedOccurrence`] keys off.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct ExtractedArbitration {
    /// The statement numbers (1-based) that assert incompatible values for the same fact; empty when
    /// nothing collides.
    #[serde(default)]
    pub(super) competing: Vec<usize>,
    /// The statement number(s) judged correct; empty when neither account is yet known to be right, so
    /// both stand.
    #[serde(default)]
    pub(super) credited: Vec<usize>,
    /// A one-line note reconciling the conflict.
    #[serde(default)]
    pub(super) statement: String,
}

/// The date-string occurrence shape the model produces — it cannot compute epoch milliseconds, so it
/// emits ISO dates (and occasionally datetimes), which [`ExtractedTime::into_temporal_ref`] maps to
/// the stored [`TemporalRef`]. Mirrors `TemporalRef`'s tags but with string dates.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExtractedTime {
    Instant(String),
    Day(String),
    Range {
        start: String,
        end: String,
    },
    Approx {
        center: String,
        fuzz_days: u32,
    },
    /// An RFC 5545 recurrence rule, e.g. `FREQ=WEEKLY;BYDAY=MO`. Only `FREQ` and `INTERVAL` are
    /// interpreted; bare English cadences like "every Monday" are dropped.
    Recurring(String),
    BeforeAfter {
        dir: String,
        anchor: String,
    },
}

impl ExtractedTime {
    /// Map the model's date strings to the stored [`TemporalRef`], or `None` if a date won't parse.
    /// A bare calendar day under `instant` becomes a `Day`: a live probe showed the model uses the
    /// two interchangeably.
    pub(super) fn into_temporal_ref(self) -> Option<TemporalRef> {
        match self {
            ExtractedTime::Instant(text) => match civil_date(&text) {
                Some(day) => Some(TemporalRef::Day(day)),
                None => Some(TemporalRef::Instant(Timestamp::from_millis(
                    time::datetime_to_millis(&text)?,
                ))),
            },
            ExtractedTime::Day(text) => civil_date(&text).map(TemporalRef::Day),
            ExtractedTime::Range { start, end } => Some(TemporalRef::Range {
                start: Timestamp::from_millis(time::date_or_datetime_to_millis(&start)?),
                end: Timestamp::from_millis(time::date_or_datetime_to_millis(&end)?),
            }),
            ExtractedTime::Approx { center, fuzz_days } => Some(TemporalRef::Approx {
                center: Timestamp::from_millis(time::date_or_datetime_to_millis(&center)?),
                fuzz_days,
            }),
            ExtractedTime::Recurring(rule) => {
                // Reject a rule this build cannot interpret (a model free-phrasing such as "every
                // Monday") rather than committing a Recurring entry that parses to no occurrence and
                // so silently never fires. Treated as unparseable, so resolve_occurrences drops it.
                let rule = Rrule(rule.into());
                time::rrule_is_supported(&rule).then_some(TemporalRef::Recurring(rule))
            }
            ExtractedTime::BeforeAfter { dir, anchor } => {
                let dir = match dir.trim().to_ascii_lowercase().as_str() {
                    "before" => Direction::Before,
                    "after" => Direction::After,
                    _ => return None,
                };
                Some(TemporalRef::BeforeAfter {
                    dir,
                    anchor: MemoryName::new(anchor),
                })
            }
        }
    }
}

/// The model's date string as a validated `Day` civil date, or `None`. A bare `YYYY-MM-DD` under
/// `instant` becomes a `Day` (the model uses the two interchangeably).
fn civil_date(text: &str) -> Option<CivilDate> {
    let date = CivilDate(text.trim().into());
    date.midnight_millis().map(|_| date)
}

/// Parse the structured-output `synthesize` reply leniently: the description is salvaged even when an
/// `occurrence` is malformed, rather than discarding the whole reply on one bad field. A smaller model
/// often mis-shapes an occurrence (flattening the nested time, or inventing one for a statement with no
/// temporal reference) while getting the description right; a strict whole-struct parse would throw all
/// of that away. Malformed occurrences are skipped, not fatal; a missing or empty description is, since
/// that is the reply's whole point. The model emits the schema as a fenced JSON block, so the object is
/// located with [`extract_json_object`] before parsing.
pub(super) fn synthesize_argument(content: &str) -> Option<SynthesizeArgs> {
    let value: serde_json::Value = serde_json::from_str(extract_json_object(content)?).ok()?;

    let description = value.get("description")?.as_str()?.trim().to_owned();
    if description.is_empty() {
        return None;
    }
    let occurrences = value
        .get("occurrences")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<ExtractedOccurrence>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Some(SynthesizeArgs {
        description,
        occurrences,
    })
}

/// Parse the focused `arbitrate` reply leniently, field by field, rather than strict-parsing the whole
/// object: a both-stand verdict credits neither side, and a model asked to "leave `credited` empty"
/// routinely expresses that by omitting the key or emitting `null` — a strict `Vec<usize>` parse throws
/// the whole conflict away over exactly the shape this call exists to record. Every field defaults, so
/// an empty `competing` (no conflict) or a null `credited` (both stand) parses cleanly; the returned
/// arbitration is then validated by [`crate::agent::turn::describe::arbitration::arbitration_event`] (>= 2 competing, a reconciling note). `None`
/// means no JSON object came back at all. The model emits the schema as a fenced JSON block, so the
/// object is located with [`extract_json_object`] before parsing.
pub(super) fn arbitrate_argument(content: &str) -> Option<ExtractedArbitration> {
    let value: serde_json::Value = serde_json::from_str(extract_json_object(content)?).ok()?;
    let statements = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_u64().map(|number| number as usize))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(ExtractedArbitration {
        competing: statements("competing"),
        credited: statements("credited"),
        statement: value
            .get("statement")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}
