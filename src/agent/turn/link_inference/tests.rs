use super::*;

#[test]
fn a_well_formed_reply_parses_into_relations_and_links() {
    let reply = serde_json::json!({
        "new_relations": [{
            "name": "authored_by",
            "inverse": "authored",
            "from_card": "many",
            "to_card": "one",
            "symmetric": false,
            "reflexive": false
        }],
        "links": [{
            "entry": 1,
            "relation": "authored_by",
            "target": "person/clara",
            "direction": "to"
        }]
    });
    let args = link_inference_argument(&reply).expect("a well-formed reply parses");
    assert_eq!(args.new_relations.len(), 1);
    assert_eq!(args.new_relations[0].name, "authored_by");
    assert_eq!(args.new_relations[0].inverse, "authored");
    assert_eq!(args.links.len(), 1);
    assert_eq!(args.links[0].target, "person/clara");
    assert_eq!(args.links[0].direction, "to");
}

#[test]
fn a_malformed_new_relation_is_skipped_while_links_survive() {
    let reply = serde_json::json!({
        "new_relations": [{ "name": "authored_by" }],
        "links": [{
            "entry": 1,
            "relation": "knows",
            "target": "person/clara",
            "direction": "to"
        }]
    });
    let args = link_inference_argument(&reply).expect("the links are salvaged");
    assert!(args.new_relations.is_empty());
    assert_eq!(args.links.len(), 1);
    assert_eq!(args.links[0].relation, "knows");
}

#[test]
fn a_malformed_link_is_skipped_while_relations_survive() {
    let reply = serde_json::json!({
        "new_relations": [{
            "name": "authored_by",
            "inverse": "authored",
            "from_card": "many",
            "to_card": "one",
            "symmetric": false,
            "reflexive": false
        }],
        "links": [{ "entry": 1, "relation": "authored_by" }]
    });
    let args = link_inference_argument(&reply).expect("the relations are salvaged");
    assert_eq!(args.new_relations.len(), 1);
    assert!(args.links.is_empty());
}

#[test]
fn a_reply_with_no_links_or_relations_parses_to_empty() {
    let reply = serde_json::json!({ "new_relations": [], "links": [] });
    let args = link_inference_argument(&reply).expect("an empty reply parses");
    assert!(args.new_relations.is_empty());
    assert!(args.links.is_empty());
}
