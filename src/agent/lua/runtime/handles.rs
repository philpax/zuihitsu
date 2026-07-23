//! Handle minting: memory handles, entry handles, link handles, and their rendering helpers.

use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::{
    event::Visibility,
    graph::RelationView,
    ids::{EntryId, MemoryId},
    memory::{
        memory_block::{EntryRef, EntrySelector, LinkDirection, LinkRef, MemoryDetails},
        search::SalientRelation,
    },
    time::format_occurrence,
};

use std::collections::{BTreeSet, HashSet};

use crate::agent::lua::{
    error::{HandleAssignmentError, HandleError, HandleKind, SearchWriteError, TaintedWriteError},
    runtime::{BlockApi, check_interpolated, render, route_error},
};
use ulid::Ulid;

/// Build a Lua handle table `{ id = "<ulid>" }` with the memory methods as its metatable index.
pub(crate) fn make_handle(lua: &Lua, id: MemoryId, metatable: &Table) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0.to_string())?;
    handle.set_metatable(Some(metatable.clone()))?;
    Ok(handle)
}

/// Build a relation result `{ name, inverse, from_card, to_card, symmetric, reflexive, description }`
/// backed by the relation metatable, so it prints readably. Cardinalities render lowercase, matching
/// the casing `links.register` accepts.
pub(crate) fn make_relation_result(
    lua: &Lua,
    view: &RelationView,
    metatable: &Table,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", view.name.as_str())?;
    table.set("inverse", view.inverse.as_str())?;
    table.set("from_card", view.from_card.as_str().to_lowercase())?;
    table.set("to_card", view.to_card.as_str().to_lowercase())?;
    table.set("symmetric", view.symmetric)?;
    table.set("reflexive", view.reflexive)?;
    table.set("description", view.description.as_str())?;
    table.set_metatable(Some(metatable.clone()))?;
    Ok(table)
}

/// Wrap a list of memory ids as a Lua sequence of handles, in order — the `calendar.*` return shape.
pub(crate) fn make_handle_list(
    lua: &Lua,
    ids: Vec<MemoryId>,
    metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    Ok(Value::Table(list))
}

/// Wrap a capped list of memory ids as a Lua sequence of handles — the `memory.list` return shape.
/// The value stays a plain sequence the agent can iterate (each element a handle, `handle.name`
/// readable), so `for _, m in ipairs(memory.list("person/")) do … end` works; the truncation note
/// rides only the *rendered* form, through the list metatable's `__tostring` reading the `more`
/// field this stores when matches were elided past the cap. So the returned value is unadorned data
/// while printing or returning it shows the `(+N more — narrow the prefix)` hint.
pub(crate) fn make_capped_handle_list(
    lua: &Lua,
    ids: Vec<MemoryId>,
    more: usize,
    metatable: &Table,
    list_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    if more > 0 {
        list.set("more", more as i64)?;
    }
    list.set_metatable(Some(list_metatable.clone()))?;
    Ok(Value::Table(list))
}

/// Render a memory's whole record to the one string `mem:details` returns: a header line (its name,
/// its description, and a `formerly …` line when it has been renamed), the live entries under a count
/// header, every link in both directions, the applied tags, and the volatility — the sections joined by
/// blank lines. Entries and links render through the *same* handle rendering `mem:entries`/`mem:links`
/// use (each row minted as its handle and stringified through its metatable), so the record reads back
/// exactly as those readers show their rows — date, stale, disputed, visibility, and teller markers on
/// an entry; `relation → name` with a dated occurrence on a link. There is no entry cap: the render is
/// the whole record, which is what lets the agent conclude it holds nothing on a topic after one look.
pub(crate) fn render_details(
    lua: &Lua,
    details: &MemoryDetails,
    entry_metatable: &Table,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<String> {
    let mut sections: Vec<String> = Vec::new();

    let mut header = details.name.clone();
    if !details.description.is_empty() {
        header.push_str(" — ");
        header.push_str(&details.description);
    }
    if !details.former_names.is_empty() {
        header.push_str(&format!("\nformerly {}", details.former_names.join(", ")));
    }
    sections.push(header);

    // The entries under a count header, each rendered as its own entry handle — the whole class read,
    // teller-private entries marked rather than omitted (this is the agent's own read).
    let count = details.entries.len();
    let mut entry_block = if count == 0 {
        "no entries".to_owned()
    } else {
        format!("{count} {}:", if count == 1 { "entry" } else { "entries" })
    };
    for entry in &details.entries {
        let handle = make_entry_handle(lua, entry, entry_metatable)?;
        entry_block.push('\n');
        entry_block.push_str(&render(lua, &Value::Table(handle)));
    }
    sections.push(entry_block);

    // Every link out of the merged identity in both directions, committed-only — the section is omitted
    // entirely when the memory has none.
    if !details.links.is_empty() {
        let mut link_block = String::from("links:");
        for link in &details.links {
            let handle = make_link_handle(lua, link, memory_metatable, link_metatable)?;
            link_block.push('\n');
            link_block.push_str(&render(lua, &Value::Table(handle)));
        }
        sections.push(link_block);
    }

    if !details.tags.is_empty() {
        let tags = details
            .tags
            .iter()
            .map(|tag| format!("#{}", tag.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        sections.push(format!("tags: {tags}"));
    }

    sections.push(format!("volatility: {}", details.volatility));

    Ok(sections.join("\n\n"))
}

/// Build a link result `{ relation, memory, name, direction, source }` backed by the link metatable,
/// so a link reader's list prints readably (`relation → name`) while each result keeps the far
/// memory as an actionable handle (`link.memory:append(...)`) and its provenance for the agent to weigh.
pub(crate) fn make_link_handle(
    lua: &Lua,
    link: &LinkRef,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("relation", link.relation.as_str())?;
    table.set("memory", make_handle(lua, link.other, memory_metatable)?)?;
    table.set("name", link.other_name.as_str())?;
    table.set("direction", link_direction_label(link.direction))?;
    table.set("source", link.source.as_str_lowercase())?;
    // The teller who asserted the relationship, for a belief-bearing relation; absent (`nil`) for a
    // link with no teller behind it, like an operator-authored `same_as`.
    if let Some(told_by) = &link.told_by {
        table.set("told_by", told_by.as_str())?;
    }
    // The far memory's representative occurrence, when it holds a dated fact — the same tagged table
    // an entry or search hit carries (e.g. `link.occurred_at.day`), so a script reads a linked event's
    // date, and the metatable's `__tostring` renders it inline on the link line.
    if let Some(occurred_at) = &link.occurred_at {
        table.set("occurred_at", lua.to_value(occurred_at)?)?;
    }
    table.set_metatable(Some(link_metatable.clone()))?;
    Ok(table)
}

/// How many of a memory's links its rendered handle lists before eliding the rest — enough to reveal
/// a hub's shape (its events, its people) without flooding the transcript when a busy topic has many.
pub(crate) const NEIGHBORHOOD_CAP: usize = 8;

/// Render a memory's link neighborhood as the compact line its handle carries: each link as
/// `relation → name` (`←` for an incoming edge), with a dated far memory's occurrence appended as
/// `[when …]` (the same phrasing a search hit's date uses), capped at [`NEIGHBORHOOD_CAP`] with a
/// `(+N more)` note. A name-and-relation list, not the targets' content: it makes the spokes legible
/// at the hub so a recall follows them rather than relaying only the hub's own entries. Empty when the
/// memory has no links, so the caller omits the line entirely.
pub(crate) fn render_neighborhood(links: &[LinkRef]) -> String {
    let mut rendered: Vec<String> = links
        .iter()
        .take(NEIGHBORHOOD_CAP)
        .map(render_link_summary)
        .collect();
    let elided = links.len().saturating_sub(NEIGHBORHOOD_CAP);
    if elided > 0 {
        rendered.push(format!("(+{elided} more)"));
    }
    rendered.join(", ")
}

/// One link on the neighborhood line: `relation → name` (or `←` for an incoming edge), plus the far
/// memory's occurrence as `[when …]` when it holds a dated fact.
fn render_link_summary(link: &LinkRef) -> String {
    let arrow = match link.direction {
        LinkDirection::Outgoing => "→",
        LinkDirection::Incoming => "←",
    };
    let mut summary = format!(
        "{} {arrow} {}",
        link.relation.as_str(),
        link.other_name.as_str()
    );
    if let Some(occurred_at) = &link.occurred_at {
        summary.push_str(&format!(" [when {}]", format_occurrence(occurred_at)));
    }
    summary
}

/// Render a search hit's salient relations as the compact segment its result line carries: each
/// relation as `relation → name` (`←` for an incoming edge), in the salience order (people first, then
/// recency), with a run of same-relation neighbours eliding the repeated label so
/// `participates_in ← person/maya, ← person/nadia` reads cleanly, and a trailing `(+N more)` when links
/// were elided past the cap. The same `relation → name` house style the neighborhood line uses, so a hit
/// passively reveals the cast already on the memory — the recognition signal that steers a search toward
/// reuse over a name-guessed duplicate. `None` when the hit carries no relations, so the caller omits the
/// segment.
pub(crate) fn render_salient_relations(
    relations: &[SalientRelation],
    more: usize,
) -> Option<String> {
    if relations.is_empty() {
        return None;
    }
    let mut rendered: Vec<String> = Vec::with_capacity(relations.len() + 1);
    let mut previous: Option<&str> = None;
    for relation in relations {
        let arrow = match relation.direction {
            LinkDirection::Outgoing => "→",
            LinkDirection::Incoming => "←",
        };
        let name = relation.other_name.as_str();
        let segment = if previous == Some(relation.relation.as_str()) {
            format!("{arrow} {name}")
        } else {
            format!("{} {arrow} {name}", relation.relation.as_str())
        };
        rendered.push(segment);
        previous = Some(relation.relation.as_str());
    }
    if more > 0 {
        rendered.push(format!("(+{more} more)"));
    }
    Some(rendered.join(", "))
}

/// Wrap a list of link refs as a Lua sequence of link results, in order — the
/// `mem:outgoing()`/`incoming()`/`links()` return shape.
pub(crate) fn make_link_handle_list(
    lua: &Lua,
    links: Vec<LinkRef>,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, link) in links.into_iter().enumerate() {
        list.set(
            index + 1,
            make_link_handle(lua, &link, memory_metatable, link_metatable)?,
        )?;
    }
    Ok(Value::Table(list))
}

/// Which way a link runs relative to the memory it was read from, as the agent-facing string a script
/// branches on — `outgoing` when the identity is the edge's source, `incoming` when its target.
fn link_direction_label(direction: LinkDirection) -> &'static str {
    match direction {
        LinkDirection::Outgoing => "outgoing",
        LinkDirection::Incoming => "incoming",
    }
}

pub(crate) fn handle_id(handle: &Table) -> mlua::Result<MemoryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(MemoryId)
        .map_err(|source| HandleError::InvalidMemoryHandle { id, source }.into())
}

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

/// The `self` a `mem:*` handle method is invoked on. Extracting it through this newtype — rather than
/// a bare `Table` — is what turns a dot-call (`memory.append(...)`, which binds the first argument to
/// `self`) into the teachable colon hint: as the method's leftmost argument, `self` is converted
/// first, so a non-table `self` fails here (with [`HandleError::MethodCalledWithDot`]) before any
/// later argument's own type error can mask it. A colon call passes the handle table, which converts
/// cleanly; the method body then resolves its id through [`handle_id`].
pub(crate) struct HandleSelf(pub(crate) Table);

impl mlua::FromLua for HandleSelf {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::Table(handle) => Ok(HandleSelf(handle)),
            other => Err(HandleError::MethodCalledWithDot {
                type_name: other.type_name(),
            }
            .into()),
        }
    }
}

/// The `__newindex` guard shared by every read-only handle metatable (memory, entry, date, and search
/// result). A handle is a view, so assigning to a field silently did nothing before this — the
/// stale-date footgun. The guard raises a teachable error naming the operation that persists the
/// change instead, tailored for `occurred_at` (the traced slip) since a date lives on an entry, not a
/// handle field. It fires only for keys absent from the raw table, which is every agent-facing field
/// (they are read through `__index` or carried as data the metamethods read), so internal setup that
/// must write a raw field uses `raw_set` to bypass it.
pub(crate) fn readonly_newindex(lua: &Lua, kind: HandleKind) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, (_, key, _): (Table, Value, Value)| {
        let field = lua
            .coerce_string(key)?
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let error = if field == "occurred_at" {
            HandleAssignmentError::OccurredAt { kind }
        } else {
            HandleAssignmentError::Other { kind, field }
        };
        Err::<(), mlua::Error>(error.into())
    })
}

/// Resolve a `:link`/`:unlink` target to its memory id. The target is normally a memory handle, but a
/// name string is accepted and looked up too — so the agent's natural call passing a name in place of
/// a handle works rather than failing the string-to-handle argument conversion, erroring, and rolling
/// the whole block back (silently dropping any co-located writes — the cause of lost sensitivity
/// markings). An unknown name is a clear error, not a silent miss.
pub(crate) fn link_target_id(api: &BlockApi, other: Value) -> mlua::Result<MemoryId> {
    match other {
        Value::Table(handle) => handle_id(&handle),
        Value::String(name) => {
            let name = name.to_string_lossy();
            match api
                .block
                .lock()
                .get(&name)
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some((id, _)) => Ok(id),
                None => Err(HandleError::UnknownLinkTarget {
                    name: name.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongLinkTargetType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Resolve an append's or link's `exclude` option to the memory ids the entry is a confidence withheld
/// from whenever they are present — the parties of a `Visibility::Exclude`. Accepts a Lua list
/// (sequence) of person handles or name strings (each resolved through [`resolve_excludee`]), a bare
/// handle or name for the single-party case, or `nil` (no exclusion). An empty list resolves to an
/// empty vec, which the block rejects as a teachable error — an exclude naming no one is just a private
/// confidence, not an exclusion. Mirrors `told_by`'s handle-or-name resolution.
pub(crate) fn resolve_exclude(
    api: &BlockApi,
    value: Value,
) -> mlua::Result<Option<BTreeSet<MemoryId>>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(_) => Ok(Some(BTreeSet::from([resolve_excludee(api, value)?]))),
        Value::Table(table) => {
            // A sequence (a `[1]` element) is a list of parties resolved one by one; a bare handle
            // table (an `id` field, no `[1]`) is a single party; anything else — an empty `{}` — yields
            // an empty set the block turns into the teachable "exclude names no one" error. The excluded
            // parties are a set: naming the same party twice, or in a different order, is the same edge.
            if !table.get::<Value>(1)?.is_nil() {
                let mut ids = BTreeSet::new();
                for element in table.sequence_values::<Value>() {
                    ids.insert(resolve_excludee(api, element?)?);
                }
                Ok(Some(ids))
            } else if table.contains_key("id")? {
                Ok(Some(BTreeSet::from([resolve_excludee(
                    api,
                    Value::Table(table),
                )?])))
            } else {
                Ok(Some(BTreeSet::new()))
            }
        }
        other => Err(HandleError::WrongExcludeeType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Resolve one `exclude` party — a memory handle or a name string — to its memory id. An unknown name,
/// or a value that is neither a handle nor a name, is a teachable error.
fn resolve_excludee(api: &BlockApi, value: Value) -> mlua::Result<MemoryId> {
    match value {
        Value::Table(handle) => handle_id(&handle),
        Value::String(name) => {
            let name = name.to_string_lossy();
            match api
                .block
                .lock()
                .get(&name)
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some((id, _)) => Ok(id),
                None => Err(HandleError::UnknownExcludee {
                    name: name.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongExcludeeType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Resolve a `memory.get` / `memory.get_or_create` argument to the name to look up. The argument is
/// normally a name string, but an existing memory handle (from `memory.list`, `memory.create`, or a
/// prior `memory.get`) is accepted too — so the natural `memory.get(h)` over a handle the agent
/// already holds works rather than failing the string-to-handle conversion and rolling the whole block
/// back. A handle resolves by its current name (through the block's pending creates, then the graph),
/// so the lookup that follows keeps read semantics identical to `memory.get("name")` — including the
/// renamed-identity affordances. An unknown handle (its id resolves to no memory) is a clear error, as
/// is any other value.
pub(crate) fn get_argument_name(api: &BlockApi, value: Value) -> mlua::Result<String> {
    match value {
        Value::String(name) => {
            let name = name.to_string_lossy();
            check_interpolated("memory name", &name)?;
            Ok(name)
        }
        Value::Table(handle) => {
            let id = handle_id(&handle)?;
            match api
                .block
                .lock()
                .handle_field(id, "name")
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some(name) => Ok(name),
                None => Err(HandleError::UnknownMemoryHandle {
                    id: id.0.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongGetArgType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Build an entry handle `{ id = "<ulid>", text = "..." }` backed by the entry metatable, so it
/// renders as its text (`__tostring` / `__concat`) yet stays addressable for `mem:supersede`.
pub(crate) fn make_entry_handle(
    lua: &Lua,
    entry: &EntryRef,
    entry_metatable: &Table,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", entry.entry_id.0.to_string())?;
    handle.set("text", entry.text.as_str())?;
    // Carried so a read renders self-describingly (see the entry metatable's `__tostring`) and so a
    // script can branch on them: `entry.visibility` ("public"/"private"), `entry.told_by` (the teller),
    // `entry.disputed` (true when the fact is under an unresolved arbitration), and `entry.occurred_at`
    // (the occurrence as the *same* tagged table `append` accepts — `{ day = "…" }` etc. — so a read
    // round-trips to a write and a script can match on `entry.occurred_at.day`, not a string it has to
    // reparse; the metatable's `__tostring` renders it for display).
    handle.set("visibility", visibility_label(&entry.visibility))?;
    handle.set("told_by", entry.teller.as_str())?;
    // The tellers standing behind the fact (its visible attestation subset, the agent skipped) — so a
    // multiply-attested fact reads `from person/erin, person/dave` and a script can branch on
    // `entry.attested_by`. Empty for an agent-only entry, where the metatable falls back to `told_by`.
    if !entry.attesters.is_empty() {
        handle.set(
            "attested_by",
            lua.create_sequence_from(entry.attesters.iter().map(String::as_str))?,
        )?;
    }
    handle.set("disputed", entry.disputed)?;
    // When set, `text` is already the withheld stub (the content never leaves the block); the flag
    // lets a script branch and lets the metatable render it as a withheld confidence, not bare text.
    handle.set("withheld", entry.withheld)?;
    // True when the fact has aged past usefulness on a high-volatility memory; the metatable renders a
    // `stale` segment so the agent hedges rather than asserting it as current.
    handle.set("stale", entry.stale)?;
    if let Some(occurred_at) = &entry.occurred_at {
        handle.set("occurred_at", lua.to_value(occurred_at)?)?;
    }
    // A retracted entry surfaces only through `mem:history`; its reason is carried so the metatable
    // renders it as a tombstone (`[retracted: …]`) and a script can branch on `entry.retracted_reason`.
    if let Some(reason) = &entry.retracted_reason {
        handle.set("retracted_reason", reason.as_str())?;
    }
    handle.set_metatable(Some(entry_metatable.clone()))?;
    Ok(handle)
}

/// The agent-facing label for an entry's visibility — `public` for freely surfaceable, `attributed`
/// for an ordinary secondhand fact the agent should weigh as relayed, and `private` for a confidence
/// (`PrivateToTeller`/`Exclude`) that only resurfaces to its teller.
pub(crate) fn visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller | Visibility::Exclude(_) => "private",
    }
}

/// Wrap a list of entry refs as a Lua sequence of entry handles, in order — the `mem:entries()` /
/// `mem:history()` return shape.
pub(crate) fn make_entry_handle_list(
    lua: &Lua,
    entries: Vec<EntryRef>,
    entry_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, entry) in entries.into_iter().enumerate() {
        list.set(index + 1, make_entry_handle(lua, &entry, entry_metatable)?)?;
    }
    Ok(Value::Table(list))
}

pub(crate) fn entry_handle_id(handle: &Table) -> mlua::Result<EntryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(EntryId)
        .map_err(|source| HandleError::InvalidEntryHandle { id, source }.into())
}

/// Resolve an entry argument to an [`EntrySelector`] the block resolves against the memory's class.
/// The argument is either an entry handle (a `{ id = … }` table read from
/// `mem:entries`/`mem:history`/`mem:append`), whose full id addresses the entry directly, or a bare
/// string — a full entry id, or a unique prefix of one read off a rendered entry line. These are the
/// forms `mem:supersede` and `mem:retract` accept, so a script can address an entry it holds a handle
/// to or one it names by (part of) its id, without re-scanning the text.
pub(crate) fn entry_selector(value: &Value) -> mlua::Result<EntrySelector> {
    match value {
        Value::Table(handle) => Ok(EntrySelector::Id(entry_handle_id(handle)?)),
        Value::String(reference) => Ok(EntrySelector::Ref(reference.to_str()?.to_owned())),
        other => Err(HandleError::WrongEntryType {
            type_name: other.type_name(),
        }
        .into()),
    }
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
