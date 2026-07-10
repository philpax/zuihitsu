//! Scaffold body tests — feature gating, dotpoint content, and template version checks.

use crate::{InstanceFeatures, event::PromptTemplateName};

use super::scaffold_body;

#[test]
fn the_scaffold_and_flush_name_the_sandbox_language_as_luau() {
    let templates = super::super::default_templates(&InstanceFeatures::default());
    let template = |name| {
        templates
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("default_templates includes {name:?}"))
    };

    let scaffold = template(PromptTemplateName::Scaffold);
    assert_eq!(
        scaffold.version, 14,
        "the scaffold is registered at v14 (v14 adds the exclude posture to the visibility point: a \
         fact everyone but a named party may know is recorded excluding that party, holding like a \
         confidence and additionally withheld whenever they are present)"
    );
    assert!(
        scaffold
            .body
            .contains("emitting Luau through the run_lua tool")
    );
    assert!(!scaffold.body.contains("emitting Lua through"));

    let flush = template(PromptTemplateName::Flush);
    assert_eq!(
        flush.version, 3,
        "the Luau-naming flush is registered at v3"
    );
    assert!(
        flush
            .body
            .contains("emitting Luau through the run_lua tool")
    );
    assert!(!flush.body.contains("emitting Lua through"));
}

#[test]
fn the_calendar_dotpoint_demonstrates_interpolation_not_concatenation() {
    let scaffold = scaffold_body(&InstanceFeatures::default());
    assert!(scaffold.contains("`Reminder for {calendar.next(\"friday\")}`"));
    assert!(!scaffold.contains("\"Reminder for \" .. calendar.next"));
}

#[test]
fn the_transcripts_dotpoint_is_gated_on_the_feature() {
    assert!(scaffold_body(&InstanceFeatures::default()).contains("convo.turn"));
    let disabled = InstanceFeatures {
        transcripts: false,
        ..Default::default()
    };
    assert!(!scaffold_body(&disabled).contains("convo.turn"));
}

#[test]
fn the_transcripts_dotpoint_teaches_only_the_token_not_a_console_url() {
    let scaffold = scaffold_body(&InstanceFeatures::default());
    assert!(scaffold.contains("[turn:<id>] token"));
    assert!(!scaffold.contains("console link"));
    assert!(!scaffold.contains("?turn="));
}

#[test]
fn the_recall_hub_walk_clause_is_gated_on_linking() {
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("follow the links the handle shows"));
    assert!(on.contains("occurred_at as it reads back"));

    let disabled = InstanceFeatures {
        linking: false,
        ..Default::default()
    };
    let off = scaffold_body(&disabled);
    assert!(!off.contains("follow the links the handle shows"));
    assert!(off.contains("occurred_at as it reads back"));
}

#[test]
fn the_scaffold_teaches_category_free_withholding() {
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("Knowing a public fact is not being someone"));
    assert!(full.contains("withhold without naming what you withhold"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("Knowing a public fact is not being someone"));
}

#[test]
fn the_scaffold_teaches_milestone_decomposition_for_dated_plans() {
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("several dated facts, not one"));
    assert!(on.contains("under its own occurred_at"));
    assert!(on.contains("independently addressable"));

    let no_calendar = InstanceFeatures {
        calendar: false,
        ..Default::default()
    };
    assert!(!scaffold_body(&no_calendar).contains("several dated facts, not one"));
}

#[test]
fn the_scaffold_teaches_search_before_creating() {
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("A name is checked exactly"));
    assert!(full.contains("never decides whether a name exists"));
    assert!(full.contains("A guessed handle that misses the existing memory mints a second"));
    assert!(full.contains("Act on results you have read"));
    assert!(full.contains("write in your next block"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("A name is checked exactly"));
    assert!(stripped.contains("Act on results you have read"));
}

#[test]
fn the_event_dotpoint_teaches_a_recurring_event_is_one_memory() {
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("A recurring or repeating gathering is ONE memory under its generic name"));
    assert!(on.contains("never a month- or date-stamped clone"));

    let no_calendar = InstanceFeatures {
        calendar: false,
        ..Default::default()
    };
    assert!(
        !scaffold_body(&no_calendar)
            .contains("A recurring or repeating gathering is ONE memory under its generic name")
    );
}

#[test]
fn the_scaffold_teaches_belief_arbitration() {
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("The record that the two accounts conflict is not yours to compose"));
    assert!(full.contains("never supersede one with the other on your own authority"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("The record that the two accounts conflict is not yours to compose"));
}

#[test]
fn the_scaffold_teaches_commit_honesty() {
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("Your reply may only claim what the commit summary shows"));
    assert!(full.contains("never confirm a write that never"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("Your reply may only claim what the commit summary shows"));
}

#[test]
fn the_recall_point_teaches_not_to_repeat_a_search() {
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("re-issuing the same search within one turn returns the same hits"));
    assert!(full.contains("answer an identity-sensitive question from a fresh read"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("re-issuing the same search within one turn returns the same hits"));
}

#[test]
fn the_merge_dotpoint_teaches_recording_and_a_rationale_before_proposing() {
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("on their current stub before you propose"));
    assert!(on.contains("state your grounds"));

    let off = scaffold_body(&InstanceFeatures {
        merging: false,
        ..Default::default()
    });
    assert!(!off.contains("on their current stub before you propose"));
}
