//! Memory API reference entries: create, get, get_or_create, search, list, append, attest, entries,
//! find_entry, history, details, supersede, retract, revise, rename, set_volatility, and the
//! always-on block.abort.

use crate::{
    agent::api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, enum_of, object},
    ids::Namespace,
};

/// The always-on memory entries plus the handle methods (append, entries, details, …).
pub(super) fn entries() -> Vec<ApiEntry> {
    let person = Namespace::Person.prefix();
    let topic = Namespace::Topic.prefix();
    let event = Namespace::Event.prefix();

    let create = AE::new("memory.create")
        .description(
            "Create a memory, optionally with a first content entry. It fails if the name is already \
             taken, so create only when you mean to make a genuinely new memory. Creation should \
             follow a check that the referent does not already exist, with the tool that fits it: \
             for a named referent, memory.list the stem or memory.get the handle (exact); for one \
             you cannot name — a topic phrased differently before — memory.search by meaning. \
             Create only when nothing matches. When unsure whether one exists, use \
             memory.get_or_create instead of guessing.",
        )
        .required(
            "name",
            AT::String,
            format!(
                "the namespaced handle, e.g. \"{person}<name>\" or \"{topic}<subject>\". Match is \
                 exact (case-sensitive), so prefer lowercase — \"{person}dave\", not \"{person}Dave\" — \
                 or one subject splits across casings"
            ),
        )
        .optional("content", AT::String, "an optional first content entry (subject to the same character limit as append)")
        .optional(
            "opts",
            object(),
            "the same overrides <memory>:append takes, applied to the first entry — including \
             visibility, exclude, occurred_at, and volatility. Seed content is an entry like any \
             other: memory.create(name, \"<summary>\") with no opts lands the summary at the \
             write-time default — Public on a non-person memory — so a guarded fact seeded this way \
             sits in the open beside its guarded siblings. On a person memory the default runs the \
             other way — a participant-told fact would land private to its teller — so an \
             unclassified seed there is refused: state the visibility. Either way, for a guarded \
             memory prefer creating it bare and appending under the guard",
        )
        .returns(AT::Handle);

    let get = AE::new("memory.get")
        .description(
            format!(
                "Fetch a memory by name or by an existing handle, or nil if there is none. Pass a \
                 handle straight through — memory.get(h) over one from memory.list or memory.create \
                 re-reads it. Read a merged identity through its \
                 canonical {person} handle, not a per-platform stub. The name must match exactly \
                 (case-sensitive); if a lookup returns nil, suspect the casing before creating a new \
                 memory. A former name still finds a renamed person: the result carries \
                 `former_handle` (the old name) and the memory carries `former_names` — they now go by \
                 `result.name`, their older entries still theirs, so answer under the current name \
                 without announcing the old one. The handle prints its name, description, and a \
                 `links:` line naming its neighborhood (each \"relation → name\", a dated target's \
                 occurrence appended), so a topic hub shows the events its decisions live on; follow \
                 those links, its own entries rarely being the whole story."
            ),
        )
        .required(
            "name",
            AT::String,
            "the memory's handle (or a former one), or an existing memory handle",
        )
        .returns(AT::Handle.optional());

    let get_or_create = AE::new("memory.get_or_create")
        .description(
            "Fetch a memory by name, or create it if there is none — the fetch-or-make idiom in one \
             call. It is for a name you have READ — off a search hit, a brief, or a handle — not one \
             you guess: a name that does not match mints a fresh duplicate under the guessed name \
             rather than finding the one you meant. So search first and pass a name you saw. If it \
             exists it is returned as-is and the content argument ignored; only an absent one is \
             created. Reserve memory.create for a deliberately new memory, where a collision should \
             error. Not an identity tool: a new name for a person you already hold is a fact on their \
             existing profile (or grounds for a rename) — get_or_create on it would mint a duplicate.",
        )
        .required(
            "name",
            AT::String,
            "the memory's handle, or an existing memory handle",
        )
        .optional(
            "content",
            AT::String,
            "an optional first entry, used only when the memory is created (ignored if it exists)",
        )
        .optional(
            "opts",
            object(),
            "the same overrides <memory>:append takes, applied to the first entry when the memory is \
             created (ignored if it exists) — including visibility, exclude, occurred_at, and \
             volatility. As with memory.create, unclassified seed content takes the write-time \
             default (Public on a non-person memory), and on a person memory an unclassified seed \
             is refused — state the visibility; for a guarded memory, prefer creating it bare and \
             appending under the guard",
        )
        .returns(AT::Handle);

    let search = AE::new("memory.search")
        .description(
            "Recall memories by meaning, across your whole memory, ranked best-first. Results are \
             filtered to who is present, so a teller-private aside appears only while its teller is \
             here (with a marker). Each result is a table { name, description, score, marker?, \
             snippet?, occurred_at?, relations? } — snippet is the matched content, so you can triage \
             even when the description is thin; occurred_at is the memory's representative date when \
             dated; relations are its most salient links (each { relation, name, direction }), so a \
             hit shows the cast already on it — letting you recognize the memory you already hold and \
             reuse it rather than making a near-duplicate. Ranked best-first means nearest in \
             meaning, not confirmed to be the referent: a top hit can be a similar but different \
             thing entirely, so check a hit's name against what you mean before writing to it: a \
             write through a hit whose name does not match the words you searched is refused, so \
             confirm it with memory.get first. A \
             hit doubles as the memory's handle — hit:details() reads every entry and occurrence \
             in full, and the other handle methods work on it directly, no memory.get round-trip.",
        )
        .required("query", AT::String, "what to look for, in natural language")
        .optional(
            "opts",
            object()
                .optional(
                    "namespace",
                    AT::String,
                    format!("restrict to a name prefix, e.g. \"{person}\""),
                )
                .optional(
                    "tags",
                    AT::String.list(),
                    "tags to prefer; a result carrying more of them ranks higher",
                )
                .optional("limit", AT::Integer, "how many results to return (default 8)"),
            "options",
        )
        .returns(AT::Object(Vec::new()).list());

    let list = AE::new("memory.list")
        .description(
            format!(
                "The existing memories whose handle begins with a name prefix — handle discovery by \
                 stem, alphabetical. Where memory.search recalls by meaning, list answers which \
                 spellings already exist: pass \"{person}dav\" to see {person}dave and {person}david \
                 before assuming a handle. Reach for it in identity work before you create or propose — \
                 list the stem to reuse an existing handle rather than minting a variant that splits \
                 the referent. Each result is a memory handle (read m.name, m.description, or call its \
                 methods); the list is capped, the remainder noted when a broad prefix matches more."
            ),
        )
        .required(
            "prefix",
            AT::String,
            format!("the name prefix to match, e.g. \"{person}\" or \"{person}dav\"; matched literally"),
        )
        .returns(AT::Handle.list());

    let append = AE::new("<memory>:append")
        .description(
            "Append a content entry. By default it is attributed to the current speaker, and an \
             aside about someone else defaults private to that speaker. An entry you record as your \
             own observation (a synthesis or a flush) has no default — set its visibility yourself, \
             public or private.",
        )
        .required("text", AT::String, "the entry text (must be under the character limit — summarize what you learned rather than pasting source content)")
        .optional(
            "opts",
            object()
                .optional(
                    "by_agent",
                    AT::Boolean,
                    "record it as your own observation instead of the speaker's",
                )
                .optional(
                    "told_by",
                    AT::Handle,
                    "attribute the entry to a specific teller other than the current speaker — a \
                     person handle (from memory.get/create) or their name as a string, which is \
                     looked up. Reach for it to record a relayed claim (\"X said that …\") with X as \
                     its source rather than the person relaying it, or on any deferred or cross-turn \
                     write where the speaker is not who the fact came from. It sets the entry's \
                     provenance, so the fact reads and is governed as that teller's — overriding \
                     by_agent and the speaker default",
                )
                .optional(
                    "visibility",
                    enum_of(["public", "private"]),
                    "force the visibility; required for an entry you author about a person",
                )
                .optional(
                    "exclude",
                    AT::Handle.list(),
                    "record it as a confidence additionally withheld whenever any named party is \
                     present — a list of person handles (or names) to keep it from, on top of the \
                     private posture (it still surfaces only to its teller's audience). Reach for it \
                     when a fact is one everyone but a specific person may know: a surprise planned \
                     for them, or something to be kept from one named individual while the others \
                     may hear it. Pass exclude instead of visibility, not alongside it — an exclude \
                     already implies private (a redundant visibility = \"private\" beside it is \
                     accepted; \"public\" or \"attributed\" is a contradiction and rejected). \
                     The recipe: create the memory bare, under a handle that reveals nothing on its \
                     own — name the occasion at most, never the plan, so \
                     \"topic/upcoming_celebration\", never a handle containing \"surprise\" or \
                     \"secret\", which tells the secret by itself since a name is never \
                     visibility-gated — then append every detail with exclude: local plan = \
                     memory.create(\"topic/upcoming_celebration\") then \
                     plan:append(\"...\", { exclude = { dave } }). Do not pass the guarded fact \
                     as create's content argument without opts: that one-liner lands the summary as \
                     an unguarded Public entry beside the excluded ones",
                )
                .optional(
                    "occurred_at",
                    object(),
                    format!(
                        "when the fact is about a real-world time (distinct from now): a tagged table, \
                         one of {{ instant = <ms> }}, {{ day = \"YYYY-MM-DD\" }}, \
                         {{ range = {{ start = <ms>, [\"end\"] = <ms> }} }} (end is a Lua keyword, \
                         quote-bracket it), {{ approx = {{ center = <ms>, fuzz_days = <n> }} }}, \
                         {{ recurring = \"<rrule>\" }}, or \
                         {{ before_after = {{ dir = \"before\" | \"after\", anchor = \"{event}...\" }} }}. \
                         A date object (calendar.today() and siblings) or a \"YYYY-MM-DD\" string is \
                         itself a valid occurred_at — pass it directly — and may also stand in for any \
                         <ms> position, its endpoint covering the whole day"
                    ),
                )
                .optional(
                    "distinct_from",
                    AT::Entry,
                    "when a near-duplicate check would otherwise fold this write into an existing \
                     entry as a corroboration, name that entry — its object, id, or a unique id \
                     prefix — to record this as a genuinely separate fact instead; the scan skips \
                     exactly that entry. Reach for it only when you mean a different fact the check \
                     mistook for the same one",
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let attest = AE::new("<memory>:attest")
        .description(
            "Stand behind an existing entry's fact as a further teller, instead of recording it \
             again. When someone independently confirms something you already hold, attest the entry \
             rather than appending a duplicate — the fact gains a corroborating teller and keeps its \
             wording. An ordinary <memory>:append already does this for you when it detects a \
             near-duplicate (returning the existing entry with a note); reach for attest when you \
             have the entry in hand and mean to corroborate it directly. The attestation is governed \
             like any confidence: its posture may sit at the entry's own audience or narrower, never \
             wider — if the fact is now openly stated where it was private, append it afresh instead.",
        )
        .required(
            "entry",
            AT::Entry,
            "the entry to stand behind — its object (from <memory>:entries or <memory>:find_entry), \
             or its id or a unique id prefix as a string",
        )
        .optional(
            "opts",
            object()
                .optional(
                    "by_agent",
                    AT::Boolean,
                    "attest it as your own observation instead of the speaker's",
                )
                .optional(
                    "told_by",
                    AT::Handle,
                    "attribute the attestation to a specific teller other than the current speaker — \
                     a person handle or their name as a string",
                )
                .optional(
                    "visibility",
                    enum_of(["public", "private"]),
                    "the attestation's posture; it may not be wider than the entry's own",
                )
                .optional(
                    "exclude",
                    AT::Handle.list(),
                    "withhold the attestation from named parties, like append's exclude",
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let entries = AE::new("<memory>:entries")
        .description(
            "The memory's live content entries, across its whole merged identity. Each is an entry \
             object — read its text with entry.text (it also prints as its text, prefixed by its id, \
             date, visibility, teller, and a disputed marker when contested), and pass the object — or \
             its id, or a unique prefix of it, shown as `id <ulid>` at the front of the printed \
             line — to <memory>:supersede or <memory>:retract to correct it. When you know the \
             wording, <memory>:find_entry(\"…\") returns the entry to correct in one step — \
             mem:retract(mem:find_entry(\"leads the volcano project\"), \"filed on the wrong \
             person\") — with no scan; or read the id off the line, e.g. \
             mem:retract(\"01KXETGK\", \"filed on the wrong person\"). entry.occurred_at, when \
             dated, is the same tagged \
             table append takes (e.g. entry.occurred_at.day), so you can match an entry by its date. \
             Capture the list — `local es = <memory>:entries()`; a bare call whose result you discard \
             returns nothing, not an empty memory. Each element renders as its own text: interpolate \
             one into a backtick string — `latest: {es[1]}` — to compose a reply, and iterate to fold \
             in several. table.concat joins only strings and numbers.",
        )
        .returns(AT::Entry.list());

    let find_entry = AE::new("<memory>:find_entry")
        .description(
            "Find the one live entry whose text contains a phrase — matched case-insensitively and \
             ignoring accents — so you can locate the entry to correct without scanning the list \
             yourself (a text scan misses on casing and paraphrase). Returns the entry object, to \
             pass straight to <memory>:retract, <memory>:supersede, or <memory>:revise — e.g. \
             mem:retract(mem:find_entry(\"leads the volcano project\"), \"filed on the wrong \
             person\") — or nil when nothing matches. If the phrase matches more than one entry it \
             is a teachable error listing the candidates: pass a longer, more distinctive phrase, or \
             address one by its id. It reads the live entries only — the same set <memory>:entries \
             shows, including ones you appended earlier in this block; a superseded entry is reached \
             through <memory>:history instead.",
        )
        .required(
            "text",
            AT::String,
            "a distinctive phrase from the entry's text — matched as a substring, ignoring case and \
             accents",
        )
        .returns(AT::Entry.optional());

    let history = AE::new("<memory>:history")
        .description(
            "The memory's entries including superseded ones, oldest first — the full record, where \
             <memory>:entries shows only the live ones. Each is an entry object (entry.text for its \
             text, and its id at the front of the printed line to address it by).",
        )
        .returns(AT::Entry.list());

    let details = AE::new("<memory>:details")
        .description(
            "The memory's whole record as one text: its header (name, description, any former names), \
             every live entry (each led by its id, to correct it by), its links in both directions, \
             its tags, and its volatility — each \
             section rendered as the dedicated readers show it. This is the way to answer \"what do I \
             hold on X\": read the canonical handle's details and the complete record is in front of \
             you in one look, so when the answer is not there — and a search or two also comes up short \
             — you can say plainly you do not hold it rather than guessing. Distinct from \
             <memory>:entries, which is only the entries; details is the whole memory at a glance.",
        )
        .returns(AT::String);

    let supersede = AE::new("<memory>:supersede")
        .description(
            "Correct a fact in place: mark an old entry superseded by a new one on the same memory. \
             Append the correction first to get the new entry object, then supersede with the old \
             entry object (from <memory>:entries) and the new one. The old entry drops from live \
             reads but stays in <memory>:history. Use this only when the same fact genuinely changed \
             — a correction or an update (often a teller revising their own earlier statement). To \
             withdraw a fact with no replacement — or to move one filed on the wrong memory — use \
             <memory>:retract instead. For two participants' conflicting accounts, do not supersede \
             one with the other: record both separately and leave the disagreement standing, to be \
             reconciled rather than silently resolved to whoever spoke last.",
        )
        .required(
            "old",
            AT::Entry,
            "the entry being replaced — its object (from <memory>:entries, or from \
             <memory>:find_entry(\"…\") for a phrase you know), or its id or a unique id prefix as a \
             string, as shown at the front of the entry's printed line",
        )
        .required(
            "new",
            AT::Entry,
            "the entry that replaces it — its object (from <memory>:append), or its id or a unique id \
             prefix as a string",
        );

    let retract = AE::new("<memory>:retract")
        .description(
            "Withdraw a fact outright, recording why — the honest fix when a fact was filed on the \
             wrong memory (a fuzzy search took the wrong person, say). Unlike <memory>:supersede \
             there is no replacement in place: the entry drops from live reads and stays in \
             <memory>:history with your reason. To put a fact on the right memory, retract it here \
             and append it afresh on the correct one — pass told_by to keep the original teller, and \
             occurred_at when you know the date. That two-step is deliberate: a fact's visibility is \
             resolved on the memory it sits on, so moving it in place would quietly change its \
             meaning; re-asserting it on the right memory is the honest correction. A reason is \
             required — an unexplained retraction is unauditable. When other tellers have \
             corroborated the fact, retracting withdraws only your own account and the fact stands on \
             theirs (you will see a note saying so); it drops entirely only once no teller stands \
             behind it.",
        )
        .required(
            "entry",
            AT::Entry,
            "the entry being withdrawn — its object (from <memory>:entries or <memory>:history, or \
             from <memory>:find_entry(\"…\") for a phrase you know), or its id or a unique id prefix \
             as a string, as shown at the front of the entry's printed line",
        )
        .required(
            "reason",
            AT::String,
            "why the fact is being withdrawn — kept in history for audit",
        );

    let revise = AE::new("<memory>:revise")
        .description(
            "Correct a fact in one call: append new_text as a new entry and supersede the old entry \
             with it, returning the new entry. Same intent as append-then-supersede without the \
             two-step, and it cannot half-apply: if the old entry is not live, the whole revision is \
             rejected, so a correction never leaves the stale value standing. Use it for a genuine \
             change to the same fact; for two people's conflicting accounts, record both separately \
             and leave the disagreement standing.",
        )
        .required(
            "old",
            AT::Entry,
            "the entry being corrected (from <memory>:entries — match it by its occurred_at or text)",
        )
        .required("new_text", AT::String, "the corrected fact's text")
        .optional(
            "opts",
            object()
                .optional("visibility", enum_of(["public", "private"]), "force the visibility")
                .optional("occurred_at", object(), "the new value's occurrence, if it is dated"),
            "the same overrides <memory>:append takes",
        )
        .returns(AT::Entry);

    let rename = AE::new("<memory>:rename")
        .description(
            format!(
                "Give this memory a new handle, keeping it the same memory — when someone changes the \
                 name they go by (a new chosen name, a married name), rename their {person} memory rather \
                 than creating a new one, which would split the person in two. It keeps all its facts, \
                 links, and history; the old name stops resolving, so use the new name from now on. \
                 Renaming onto a handle that already belongs to a different memory is an error — that is \
                 two people, not a rename. Platform handles ({person}<user>@<platform>) are the \
                 connectors' own and follow the platform: rename the person's bare {person}<name> \
                 profile, never a platform handle."
            ),
        )
        .required("name", AT::String, format!("the new handle, e.g. \"{person}sarah\""));

    let set_volatility = AE::new("<memory>:set_volatility")
        .description(
            "Set a memory's volatility — how fast its facts drift. \"high\" for fast-changing facts \
             (a current role, where someone is, what they are working on), \"medium\" the default, \
             \"low\" for durable facts like a name. A high-volatility memory surfaces later flagged \
             `stale — no newer entry` once it ages out: possibly out of date with nothing to replace \
             it, so hedge or reconfirm rather than hunting for a fresher version.",
        )
        .required(
            "level",
            enum_of(["low", "medium", "high"]),
            "the volatility level",
        );

    vec![
        create,
        get,
        get_or_create,
        search,
        list,
        append,
        attest,
        entries,
        find_entry,
        details,
        history,
        supersede,
        retract,
        revise,
        rename,
        set_volatility,
    ]
}

/// The merge entry (`<memory>:propose_merge`), gated on the `merging` feature.
pub(super) fn merge_entries() -> Vec<ApiEntry> {
    let person = Namespace::Person.prefix();
    let propose_merge = AE::new("<memory>:propose_merge")
        .description(
            format!(
                "Record that this {person} stub and another are the same human across platforms, for \
                 the operator to weigh and confirm. This does not merge them and surfaces nothing on its \
                 own — it is your judgment, offered to the operator against the independently-recorded \
                 facts, and nothing merges until the operator confirms it. Propose only from what you \
                 already hold about each, never from claims made to convince you in the moment; you \
                 cannot merge by asserting same_as yourself. Pass opts.rationale to state why you think \
                 they match (the observed coincidence — a shared wedding, the same volcanology trip), \
                 weighed against the entries on both stubs."
            ),
        )
        .required("other", AT::Handle, format!("the other {person} stub"))
        .optional(
            "opts",
            object().optional(
                "rationale",
                AT::String,
                "why you think the two are the same person — the observed coincidence, stated as your \
                 grounds for the operator to weigh against the recorded facts",
            ),
            "options",
        );
    vec![propose_merge]
}

/// The `block.abort` entry — always on, infrastructure.
pub(super) fn block_entries() -> Vec<ApiEntry> {
    let abort = AE::new("block.abort")
        .description("Discard everything this block buffered and end it, recording the reason.")
        .optional("reason", AT::String, "why the block was abandoned");
    vec![abort]
}

/// The `turn.skip` entry — always on, infrastructure.
pub(super) fn turn_entries() -> Vec<ApiEntry> {
    let skip = AE::new("turn.skip")
        .description(
            "End the turn silently, committing this block's writes. Unlike block.abort (which \
             discards), turn.skip keeps what you wrote — use it when you gathered information and \
             decided the message does not need a response. No further model step runs.",
        )
        .optional("reason", AT::String, "why the turn was skipped");
    vec![skip]
}
