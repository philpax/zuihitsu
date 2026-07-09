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
        "<memory>:link",
        "<memory>:unlink",
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
