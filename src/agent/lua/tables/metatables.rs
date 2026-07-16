//! Handle and result metatables minted for the per-block Lua globals: memory and entry
//! handles, date objects, and the search, tag, link, relation, and turn-window result rows.

use crate::{agent::lua::tables::*, time::TemporalRef};

/// The metatable backing entry handles: `__tostring` and `__concat` render the handle as its
/// `text`, so a content read stays ergonomic (printable, concatenable) while the handle remains an
/// addressable entry for `mem:supersede`.
pub(crate) fn entry_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    // An entry renders self-describingly: its text prefixed by its id and by what governs reading it
    // — when the fact occurs (if dated), a `disputed` marker when it is under an unresolved
    // arbitration, the visibility, and who it came from, e.g.
    // "[01JQ… · 2027-03-15 · disputed · private · from person/erin] …". The id leads the bracket so a
    // read shows the stable handle to correct the entry by (pass it — or a unique prefix of it — to
    // `mem:supersede`/`mem:retract`); text-scanning to re-find an entry misses on case and paraphrase.
    // So printing a memory's entries shows at a glance which entry to address, when a dated fact
    // happens, which are contested, which are confidences to hold, and whose they are — rather than
    // bare text whose id, date, and provenance the agent has to reconstruct (or search for) separately.
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let text = this.get::<String>("text")?;
            let mut segments = Vec::new();
            // The full id leads the bracket, labelled so a bare ULID reads as the addressing
            // affordance it is, not opaque metadata — `mem:supersede`/`mem:retract` take it (or a
            // unique prefix). Rendered in full, never shortened: same-block entries share their
            // ULID timestamp prefix, so a shortened render would be exactly the ambiguous case.
            if let Some(id) = this.get::<Option<String>>("id")? {
                segments.push(format!("id {id}"));
            }
            // `occurred_at` is the structured tagged table; render it back to a date for display.
            let occurred = this.get::<Value>("occurred_at")?;
            if !occurred.is_nil()
                && let Ok(temporal) = lua.from_value::<TemporalRef>(occurred)
            {
                segments.push(time::format_occurrence(&temporal));
            }
            if this.get::<Option<bool>>("disputed")?.unwrap_or(false) {
                segments.push("disputed".to_owned());
            }
            if this.get::<Option<bool>>("stale")?.unwrap_or(false) {
                segments.push(crate::decay::STALE_LABEL.to_owned());
            }
            // A retracted entry (surfaced only by history) leads with its tombstone reason, so a
            // history read shows *why* a withdrawn fact was withdrawn beside it.
            if let Some(reason) = this.get::<Option<String>>("retracted_reason")? {
                segments.push(format!("retracted: {reason}"));
            }
            if let (Some(visibility), Some(teller)) = (
                this.get::<Option<String>>("visibility")?,
                this.get::<Option<String>>("told_by")?,
            ) {
                segments.push(format!("{visibility} · from {teller}"));
            }
            Ok(if segments.is_empty() {
                text
            } else {
                format!("[{}] {text}", segments.join(" · "))
            })
        })?,
    )?;
    metatable.set(
        "__concat",
        lua.create_function(|lua, (left, right): (Value, Value)| {
            Ok(format!(
                "{}{}",
                value_text(lua, &left)?,
                value_text(lua, &right)?
            ))
        })?,
    )?;
    // An entry is a read-only view (its fields carry the read; a change is an append/revise/supersede,
    // not a field write), so assigning to one raises the teachable error rather than silently doing
    // nothing.
    metatable.set("__newindex", readonly_newindex(lua, HandleKind::Entry)?)?;
    Ok(metatable)
}

/// The metatable backing the date objects `calendar` constructs. `__tostring` and `__concat` render
/// the ISO day (so a date prints and concatenates as `"YYYY-MM-DD"` — `"Reminder for " .. friday`
/// works — rather than erroring as a bare table), and `:to_string()` returns that same day. The other
/// methods are calendar-correct arithmetic returning new date objects (`:add_days`, `:add_weeks`,
/// `:add_months`), plus `:weekday()`. A date object is `{ day = "YYYY-MM-DD" }`, so it doubles as an
/// `occurred_at` value — the runtime does the date math the model would otherwise slip on.
pub(crate) fn date_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| this.get::<String>("day"))?,
    )?;
    metatable.set(
        "__concat",
        lua.create_function(|lua, (left, right): (Value, Value)| {
            Ok(format!(
                "{}{}",
                date_text(lua, &left)?,
                date_text(lua, &right)?
            ))
        })?,
    )?;
    let methods = lua.create_table()?;
    // :to_string() — the ISO day as a string, the explicit form of the `__tostring`/`__concat`
    // rendering for a script that wants the string in hand.
    methods.set(
        "to_string",
        lua.create_function(|_, this: Table| this.get::<String>("day"))?,
    )?;
    // :add_days(n) / :add_weeks(n) — shift by whole days (a UTC day plus whole days is exact).
    for (name, per) in [("add_days", 1i64), ("add_weeks", 7)] {
        let mt = metatable.clone();
        methods.set(
            name,
            lua.create_function(move |lua, (this, count): (Table, i64)| {
                let day = this.get::<String>("day")?;
                let shifted = time::add_days(&day, count.saturating_mul(per))
                    .ok_or_else(|| CalendarError::InvalidDay { input: day.clone() })?;
                make_date(lua, shifted, &mt)
            })?,
        )?;
    }
    // :add_months(n) — calendar arithmetic, clamping a day past the target month's length.
    let mt = metatable.clone();
    methods.set(
        "add_months",
        lua.create_function(move |lua, (this, count): (Table, i64)| {
            let day = this.get::<String>("day")?;
            let shifted = time::add_months(&day, count)
                .ok_or_else(|| CalendarError::InvalidDay { input: day.clone() })?;
            make_date(lua, shifted, &mt)
        })?,
    )?;
    // :weekday() — the day's weekday name.
    methods.set(
        "weekday",
        lua.create_function(|_, this: Table| {
            let day = this.get::<String>("day")?;
            Ok(time::weekday(&day)
                .ok_or_else(|| CalendarError::InvalidDay { input: day.clone() })?)
        })?,
    )?;
    metatable.set("__index", methods)?;
    // A date object is a read-only value (arithmetic returns a *new* date); assigning to its `day`
    // field is a silent no-op, so the guard raises the teachable error instead.
    metatable.set("__newindex", readonly_newindex(lua, HandleKind::Date)?)?;
    Ok(metatable)
}

/// The metatable backing `memory.search` result objects: `__tostring` renders one as a readable
/// line (name, score, description, the matched-content snippet, the representative occurrence, the
/// salient relations, and any teller-private marker), so returning the result list reads back as text
/// rather than `<table>` while each result keeps its fields for the agent to inspect (`result.name` to
/// fetch, `result.score` to weigh, `result.occurred_at.day` to read a date, `result.relations` to read
/// the cast). The snippet is the content that produced the hit, so a result stays triageable even when
/// the description is stale or empty; the occurrence carries a scheduled or dated fact's date so a recall
/// relayed from the line keeps the *when*; the relations carry the memory's most salient links, so the
/// hit reveals who already participates in it — the recognition signal that steers a search toward reuse.
pub(super) fn search_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let name: String = this.get("name")?;
            let description: String = this.get("description")?;
            let score: f32 = this.get("score")?;
            let marker: Option<String> = this.get("marker")?;
            let snippet: Option<String> = this.get("snippet")?;
            let mut line = format!("{name} (score {score:.2})");
            if !description.is_empty() {
                line.push_str(" — ");
                line.push_str(&description);
            }
            if let Some(snippet) = snippet.filter(|s| !s.is_empty()) {
                line.push_str(&format!(" match: \"{snippet}\""));
            }
            // The representative occurrence renders inline (like an entry's date on read), so a recall
            // that relays the hit line still carries a scheduled or dated fact's date. The stored value
            // is the structured tagged table; render it back to a date for display.
            let occurred = this.get::<Value>("occurred_at")?;
            if !occurred.is_nil()
                && let Ok(temporal) = lua.from_value::<TemporalRef>(occurred)
            {
                line.push_str(&format!(" [when {}]", time::format_occurrence(&temporal)));
            }
            // The salient relations (its cast) render inline as `relation → name`, pre-rendered when the
            // result was built, so the printed hit reveals who already participates in this memory —
            // the recognition signal that steers a recall toward reuse over a name-guessed duplicate.
            let relations_line: Option<String> = this.get("relations_line")?;
            if let Some(relations_line) = relations_line.filter(|line| !line.is_empty()) {
                line.push_str(" — ");
                line.push_str(&relations_line);
            }
            if let Some(marker) = marker {
                line.push(' ');
                line.push_str(&marker);
            }
            Ok(line)
        })?,
    )?;
    // A search result is a read-only row; assigning to its fields does nothing, so the guard raises
    // the teachable error naming the operation that persists a change.
    metatable.set(
        "__newindex",
        readonly_newindex(lua, HandleKind::SearchResult)?,
    )?;
    // A hit concatenates as its rendered line, mirroring how it prints.
    metatable.set("__concat", concat_via_tostring(lua)?)?;
    Ok(metatable)
}

/// The metatable backing `tags.list` result objects: `__tostring` renders one as `name — purpose
/// (N uses)`, so the vocabulary reads back as text rather than `<table>` while each result keeps
/// its `name`, `description`, and `count` fields.
pub(super) fn tag_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let name: String = this.get("name")?;
            let description: String = this.get("description")?;
            let count: i64 = this.get("count")?;
            let uses = if count == 1 {
                "1 use".to_owned()
            } else {
                format!("{count} uses")
            };
            let mut line = name;
            if !description.is_empty() {
                line.push_str(" — ");
                line.push_str(&description);
            }
            line.push_str(&format!(" ({uses})"));
            Ok(line)
        })?,
    )?;
    Ok(metatable)
}

/// The metatable backing the link results `mem:outgoing`/`incoming`/`links` return: `__tostring`
/// renders one as `relation → name` (outgoing) or `relation ← name` (incoming) — with a dated far
/// memory's occurrence appended as `[when …]` (the same phrasing a search hit uses) — so a reader's
/// list reads back as readable relationships that keep the linked event's *when*, while each result
/// keeps its `relation`, `memory` (the far memory as a handle), `name`, `direction`, `source`, and
/// `occurred_at` fields for the agent to inspect and act on.
pub(super) fn link_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let relation: String = this.get("relation")?;
            let name: String = this.get("name")?;
            let direction: String = this.get("direction")?;
            let arrow = if direction == "incoming" {
                "←"
            } else {
                "→"
            };
            let mut line = format!("{relation} {arrow} {name}");
            // The far memory's occurrence renders inline (like a search hit's date), so a link to a
            // dated event carries *when* on the line. The stored value is the structured tagged table;
            // render it back to a date for display.
            let occurred = this.get::<Value>("occurred_at")?;
            if !occurred.is_nil()
                && let Ok(temporal) = lua.from_value::<TemporalRef>(occurred)
            {
                line.push_str(&format!(" [when {}]", time::format_occurrence(&temporal)));
            }
            Ok(line)
        })?,
    )?;
    // `"- " .. link` composes the link's rendered line — the join the agent writes when listing a
    // memory's relationships — rather than erroring as a bare table.
    metatable.set("__concat", concat_via_tostring(lua)?)?;
    Ok(metatable)
}

/// The metatable backing `links.list`/`links.get` result objects: `__tostring` renders one as
/// `name / inverse — from-to[, symmetric][, reflexive]`, so the registry reads back as text while
/// each result keeps its fields.
pub(super) fn relation_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let name: String = this.get("name")?;
            let inverse: String = this.get("inverse")?;
            let from_card: String = this.get("from_card")?;
            let to_card: String = this.get("to_card")?;
            let symmetric: bool = this.get("symmetric")?;
            let reflexive: bool = this.get("reflexive")?;
            let description: String = this.get("description")?;
            let mut line = format!("{name} / {inverse} — {from_card}-to-{to_card}");
            if symmetric {
                line.push_str(", symmetric");
            }
            if reflexive {
                line.push_str(", reflexive");
            }
            line.push_str(&format!(": {description}"));
            Ok(line)
        })?,
    )?;
    Ok(metatable)
}

/// The metatable backing a `memory.list` result: `__tostring` renders the handle lines (each memory
/// handle as its own `name — description`) and, when matches were elided past the cap, appends a
/// `(+N more — narrow the prefix)` note read from the list's `more` field. The list stays a plain
/// sequence of handles the agent iterates; only its rendered form carries the note, so a returned or
/// printed list reveals the truncation while `ipairs` still walks the handles.
pub(super) fn handle_list_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let mut lines: Vec<String> = this
                .clone()
                .sequence_values::<Value>()
                .filter_map(Result::ok)
                .map(|value| render(lua, &value))
                .collect();
            if let Some(more) = this.get::<Option<i64>>("more")?.filter(|more| *more > 0) {
                lines.push(format!("(+{more} more — narrow the prefix)"));
            }
            Ok(lines.join("\n"))
        })?,
    )?;
    Ok(metatable)
}

/// The metatable backing a `convo.turn` window line: `__tostring` renders it as `[at] speaker: text`,
/// with a `»` marker on the focal turn, so a window reads back as a transcript excerpt with the linked
/// moment called out.
pub(super) fn turn_line_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let at: String = this.get("at")?;
            let speaker: String = this.get("speaker")?;
            let text: String = this.get("text")?;
            let focused: bool = this.get("focused")?;
            let marker = if focused { "» " } else { "  " };
            Ok(format!("{marker}[{at}] {speaker}: {text}"))
        })?,
    )?;
    Ok(metatable)
}

/// The metatable backing the `convo.turn` result: `__tostring` renders its `window` as the joined
/// transcript lines, so `return convo.turn(id)` reads back as the exchange around the linked moment.
pub(super) fn turn_window_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let window: Table = this.get("window")?;
            let lines: Vec<String> = window
                .sequence_values::<Value>()
                .filter_map(Result::ok)
                .map(|line| render(lua, &line))
                .collect();
            Ok(lines.join("\n"))
        })?,
    )?;
    Ok(metatable)
}
