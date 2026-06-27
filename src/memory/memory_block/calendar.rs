//! Calendar queries: memories with a concrete or recurring occurrence, in a window or on a day.

use std::collections::BTreeSet;

use crate::{
    ids::MemoryId,
    time::{self, TemporalRef, Timestamp},
};

use super::{DEFAULT_UPCOMING_DAYS, MemoryBlock, MemoryError};

impl MemoryBlock {
    /// The current time off the engine clock — the anchor the `calendar` date constructors build
    /// relative dates on, so the agent names an operation rather than computing a date.
    pub fn now(&self) -> Timestamp {
        self.engine.clock.now()
    }

    /// Memories with a concrete occurrence within `within` of now (e.g. `"7 days"`, `"2 weeks"`;
    /// defaults to 7 days), soonest first (spec §Calendar). A read, so the results are touched.
    pub fn upcoming(&mut self, within: Option<&str>) -> Result<Vec<MemoryId>, MemoryError> {
        let within_millis = match within {
            Some(text) => time::parse_duration_millis(text)
                .ok_or_else(|| MemoryError::BadCalendarArg(text.to_owned()))?,
            None => DEFAULT_UPCOMING_DAYS * time::MILLIS_PER_DAY,
        };
        let now = self.engine.clock.now().as_millis();
        self.occurrence_memories(
            Timestamp::from_millis(now),
            Timestamp::from_millis(now.saturating_add(within_millis)),
        )
    }

    /// Memories with a concrete occurrence on the civil day `date` (`YYYY-MM-DD`).
    pub fn on(&mut self, date: &str) -> Result<Vec<MemoryId>, MemoryError> {
        let (from, to) =
            time::day_window(date).ok_or_else(|| MemoryError::BadCalendarArg(date.to_owned()))?;
        self.occurrence_memories(Timestamp::from_millis(from), Timestamp::from_millis(to))
    }

    /// Memories that carry a recurring occurrence — a listing; instances are not expanded yet.
    pub fn recurring(&mut self) -> Result<Vec<MemoryId>, MemoryError> {
        let ids: Vec<MemoryId> = self
            .engine
            .graph
            .lock()
            .recurring_memories()?
            .into_iter()
            .map(|memory| memory.id)
            .collect();
        for id in &ids {
            self.touched.insert(*id);
        }
        Ok(ids)
    }

    /// The distinct memories with an occurrence in `[from, to]`, soonest first, touched as reads —
    /// both concrete occurrences and the next in-window instance of a recurring entry (spec §Recurring
    /// materialization), merged and ordered by instant so a weekly standup interleaves with one-offs.
    fn occurrence_memories(
        &mut self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<MemoryId>, MemoryError> {
        let mut items: Vec<(Timestamp, MemoryId)> = Vec::new();
        {
            let graph = self.engine.graph.lock();
            for (memory, entry) in graph.occurrences_in_window(from, to)? {
                if let Some(sort) = entry.occurred_sort {
                    items.push((sort, memory.id));
                }
            }
            for (instant, memory) in graph.recurring_in_window(from, to)? {
                items.push((instant, memory.id));
            }
        }
        items.sort_by_key(|(instant, _)| *instant);

        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for (_, id) in items {
            if seen.insert(id) {
                ordered.push(id);
            }
        }
        for id in &ordered {
            self.touched.insert(*id);
        }
        Ok(ordered)
    }

    /// A write's `occurred_at` is one this build can interpret, or a teachable error. A `Recurring`
    /// ref must carry a rule the wake-up scheduler can arm (a supported `FREQ`); a free-phrased cadence
    /// such as "every Monday" is rejected here rather than becoming a silent dud. The other variants
    /// carry no rule to misread.
    pub(super) fn validate_occurred_at(
        occurred_at: Option<&TemporalRef>,
    ) -> Result<(), MemoryError> {
        match occurred_at {
            Some(TemporalRef::Recurring(rule)) if !time::rrule_is_supported(rule) => {
                Err(MemoryError::UnsupportedRecurrence(rule.0.to_string()))
            }
            _ => Ok(()),
        }
    }
}
