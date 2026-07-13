use super::api_reference;
use crate::InstanceFeatures;

/// The call names of a feature's entries, in order, for a readable diff on failure.
fn names(features: &InstanceFeatures) -> Vec<String> {
    api_reference(features)
        .iter()
        .map(|entry| entry.call.clone())
        .collect()
}

#[test]
fn disabling_linking_omits_every_link_entry() {
    let features = InstanceFeatures {
        linking: false,
        ..Default::default()
    };
    let entries = names(&features);
    // The write and read sides of linking both vanish.
    for name in [
        "links.create",
        "links.remove",
        "<memory>:outgoing",
        "<memory>:incoming",
        "<memory>:links",
        "links.register",
        "links.list",
        "links.get",
    ] {
        assert!(
            !entries.contains(&name.to_owned()),
            "{name:?} should be absent"
        );
    }
    // Memory and context remain.
    assert!(entries.contains(&"memory.create".to_owned()));
    assert!(entries.contains(&"context.current".to_owned()));
}

#[test]
fn browsing_gates_the_web_entry() {
    // On by default: web.markdown is described.
    assert!(names(&InstanceFeatures::default()).contains(&"web.markdown".to_owned()));
    // Off: the reference omits it, so the prompt never describes a call the runtime will not install.
    let features = InstanceFeatures {
        browsing: false,
        ..Default::default()
    };
    assert!(!names(&features).contains(&"web.markdown".to_owned()));
    // The rest of the surface is unaffected.
    assert!(names(&features).contains(&"memory.create".to_owned()));
}

#[test]
fn disabling_merging_omits_propose_merge() {
    let features = InstanceFeatures {
        merging: false,
        ..Default::default()
    };
    let entries = names(&features);
    assert!(!entries.contains(&"<memory>:propose_merge".to_owned()));
}

#[test]
fn propose_merge_documents_the_rationale_option() {
    // The propose_merge entry documents opts.rationale and when to state it (the observed
    // coincidence), so the agent learns to pass its stated grounds for the adjudicator to weigh.
    let propose_merge = api_reference(&InstanceFeatures::default())
        .into_iter()
        .find(|entry| entry.call == "<memory>:propose_merge")
        .expect("propose_merge is present by default");
    assert!(
        propose_merge.doc.contains("opts.rationale"),
        "the description should mention opts.rationale: {}",
        propose_merge.doc
    );
    let rationale_param = propose_merge
        .params
        .iter()
        .find(|param| param.name == "opts")
        .and_then(|opts| match &opts.ty {
            crate::agent::api_doc::ApiType::Object(fields) => {
                fields.iter().find(|field| field.name == "rationale")
            }
            _ => None,
        })
        .expect("opts carries a rationale field");
    assert!(
        rationale_param
            .doc
            .contains("why you think the two are the same person"),
        "the rationale param should say when to use it: {}",
        rationale_param.doc
    );
}

#[test]
fn create_teaches_search_before_creating() {
    // memory.create teaches that creation follows an existence check with the tool that fits the
    // referent: exact lookups for a name, memory.search by meaning for one it cannot name.
    let create = api_reference(&InstanceFeatures::default())
        .into_iter()
        .find(|entry| entry.call == "memory.create")
        .expect("memory.create is present");
    assert!(
        create
            .doc
            .contains("Creation should follow a check that the referent does not already exist"),
        "create should teach search-first: {}",
        create.doc
    );
    assert!(
        create.doc.contains("Create only when nothing matches"),
        "create should teach reuse of a match: {}",
        create.doc
    );
}

#[test]
fn get_or_create_is_for_a_read_name_not_a_guess() {
    // memory.get_or_create is repositioned: it is for a name you have READ (a search hit, a brief,
    // a handle), not one you guess — a guessed name that misses mints a duplicate.
    let get_or_create = api_reference(&InstanceFeatures::default())
        .into_iter()
        .find(|entry| entry.call == "memory.get_or_create")
        .expect("memory.get_or_create is present");
    assert!(
        get_or_create.doc.contains("a name you have READ"),
        "get_or_create should be for a read name: {}",
        get_or_create.doc
    );
    assert!(
        get_or_create
            .doc
            .contains("mints a fresh duplicate under the guessed name"),
        "get_or_create should warn that a guessed miss duplicates: {}",
        get_or_create.doc
    );
}

#[test]
fn search_documents_the_relations_field() {
    // The memory.search result shape documents `relations` and why it is there — recognizing the
    // memory you already hold, to reuse it rather than make a near-duplicate.
    let search = api_reference(&InstanceFeatures::default())
        .into_iter()
        .find(|entry| entry.call == "memory.search")
        .expect("memory.search is present");
    assert!(
        search
            .doc
            .contains("marker?, snippet?, occurred_at?, relations?"),
        "the result shape should list relations?: {}",
        search.doc
    );
    assert!(
        search.doc.contains("recognize the memory you already hold"),
        "search should say why relations are there: {}",
        search.doc
    );
}

#[test]
fn append_documents_the_exclude_option() {
    // The append entry documents the exclude opt — a confidence additionally withheld whenever a
    // named party is present — and that it is mutually exclusive with visibility.
    let append = api_reference(&InstanceFeatures::default())
        .into_iter()
        .find(|entry| entry.call == "<memory>:append")
        .expect("append is present");
    let exclude = append
        .params
        .iter()
        .find(|param| param.name == "opts")
        .and_then(|opts| match &opts.ty {
            crate::agent::api_doc::ApiType::Object(fields) => {
                fields.iter().find(|field| field.name == "exclude")
            }
            _ => None,
        })
        .expect("opts carries an exclude field");
    assert!(
        exclude.doc.contains("withheld") && exclude.doc.contains("Mutually exclusive"),
        "the exclude param should describe the posture and the conflict: {}",
        exclude.doc
    );
}

#[test]
fn create_and_get_or_create_document_the_first_entry_overrides() {
    // Both creation calls document their opts parameter — the same overrides append takes, applied
    // to the first entry — so a guarded seed entry is classified at creation instead of taking the
    // write-time default (the Public-copy leak beside an excluded sibling).
    let reference = api_reference(&InstanceFeatures::default());
    for call in ["memory.create", "memory.get_or_create"] {
        let entry = reference
            .iter()
            .find(|entry| entry.call == call)
            .unwrap_or_else(|| panic!("{call} is present"));
        let opts = entry
            .params
            .iter()
            .find(|param| param.name == "opts")
            .unwrap_or_else(|| panic!("{call} documents an opts param"));
        assert!(
            opts.doc.contains("<memory>:append") && opts.doc.contains("exclude"),
            "{call}'s opts should defer to append's overrides and name exclude: {}",
            opts.doc
        );
    }
}

#[test]
fn disabling_transcripts_omits_convo_turn() {
    let features = InstanceFeatures {
        transcripts: false,
        ..Default::default()
    };
    assert!(!names(&features).contains(&"convo.turn".to_owned()));
    // On by default, it is present.
    assert!(names(&InstanceFeatures::default()).contains(&"convo.turn".to_owned()));
}

#[test]
fn disabling_calendar_omits_every_calendar_entry() {
    let features = InstanceFeatures {
        calendar: false,
        ..Default::default()
    };
    let entries = names(&features);
    assert!(!entries.contains(&"calendar.today".to_owned()));
    assert!(!entries.contains(&"<date>:add_days".to_owned()));
}

#[test]
fn append_documents_the_character_limit() {
    // The append entry's text param documents the character limit, so the agent learns to
    // summarize what it learned rather than pasting source content verbatim.
    let append = api_reference(&InstanceFeatures::default())
        .into_iter()
        .find(|entry| entry.call == "<memory>:append")
        .expect("append is present");
    let text_param = append
        .params
        .iter()
        .find(|param| param.name == "text")
        .expect("append has a text param");
    assert!(
        text_param.doc.contains("character limit"),
        "the text param should mention the character limit: {}",
        text_param.doc
    );
}
