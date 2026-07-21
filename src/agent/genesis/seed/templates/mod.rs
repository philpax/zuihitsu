//! The default prompt templates, embedded from markdown and assembled feature-gated.
//!
//! A feature-gated point is dropped when its feature is off, so the prompt never teaches a practice
//! the runtime rejects — the scaffold is one of three gates (with the Lua registration and the API
//! reference) that move in lockstep.
//!
//! Editing a body changes the baked scaffold, so bump that template's version.

use crate::{
    InstanceFeatures,
    event::PromptTemplateName,
    ids::{MemoryName, Namespace},
};

use crate::agent::genesis::TemplateDef;

pub(crate) fn default_templates(features: &InstanceFeatures) -> Vec<TemplateDef> {
    vec![
        TemplateDef {
            name: PromptTemplateName::Scaffold,
            version: 23,
            body: scaffold_body(features),
        },
        TemplateDef {
            name: PromptTemplateName::DescriptionRegen,
            version: 1,
            body: body_of(include_str!("synthesis/description_regen.md")),
        },
        // The body leads with the omit-default because over-resolution is the dangerous direction: a
        // statement stamped with a fabricated now-relative date reads back as fact, while an untimed
        // one merely sends the reader to the entry.
        TemplateDef {
            name: PromptTemplateName::TemporalExtraction,
            version: 5,
            body: body_of(include_str!("synthesis/temporal_extraction.md")),
        },
        TemplateDef {
            name: PromptTemplateName::Flush,
            version: 3,
            body: flush_template_body(),
        },
        TemplateDef {
            name: PromptTemplateName::Imprint,
            version: 3,
            body: body_of(include_str!("turn/imprint.md")),
        },
        // A coined directional relation is easy to link the wrong way round, so the body has the
        // model express each link as a subject–relation–object sentence — the direction is carried
        // by the sentence itself, not by a separate flag the model would have to reason out.
        TemplateDef {
            name: PromptTemplateName::LinkInference,
            version: 5,
            body: body_of(include_str!("synthesis/link_inference.md")),
        },
    ]
}

/// The Flush template body. A flush turn — whether the pre-compaction end-flush or a mid-session
/// checkpoint — writes durable working state to memory with the turn's own visibility discipline.
/// It teaches no session-lifetime link flag: the working set carried across a compaction seam is
/// platform-derived (the session's touched set), so the agent has no such flags to manage on the
/// semantic graph.
pub(crate) fn flush_template_body() -> String {
    body_of(include_str!("turn/flush.md"))
}

/// Assemble the scaffold body from the shared preamble and the guidance points, feature-gated points
/// included only when their feature is on. Each point renders as its own bullet.
fn scaffold_body(features: &InstanceFeatures) -> String {
    /// The transcript point's reconstruction clause leans on link-following, which is the `linking`
    /// feature: with linking on it walks one hop out to the surrounding nodes, with it off it relies on
    /// search hits alone.
    fn transcript_point(features: &InstanceFeatures) -> String {
        let reconstruct = if features.linking {
            body_of(include_str!("turn/scaffold/reconstruct_linked.md"))
        } else {
            body_of(include_str!("turn/scaffold/reconstruct_plain.md"))
        };
        include_str!("turn/scaffold/transcript.md").replace("{{reconstruct}}", &reconstruct)
    }

    // `recall_point` and `transcript_point` resolve their linking-gated inline fragments before the
    // shared `render` pass, so they arrive as owned strings rather than `include_str!` literals; bind
    // them here so the point list can borrow them and `render` every point uniformly at assembly.
    let recall = recall_point(features);
    let transcript = features.transcripts.then(|| transcript_point(features));

    let mut points: Vec<&str> = vec![];
    points.extend([
        include_str!("turn/scaffold/namespace_kinds.md"),
        recall.as_str(),
    ]);
    // The merge dotpoint teaches `:propose_merge` — include it only when merging is on.
    if features.merging {
        points.push(include_str!("turn/scaffold/merge.md"));
    }
    points.extend([
        include_str!("turn/scaffold/impersonation.md"),
        include_str!("turn/scaffold/one_person.md"),
        include_str!("turn/scaffold/remember_now.md"),
    ]);
    // The event and calendar-date dotpoints teach `occurred_at` recurring rules and `calendar.*` date
    // arithmetic — include them only when calendar is on.
    if features.calendar {
        points.extend([
            include_str!("turn/scaffold/event.md"),
            include_str!("turn/scaffold/calendar_dates.md"),
        ]);
    }
    points.push(include_str!("turn/scaffold/record.md"));
    // The web-fetch dotpoint teaches `web.markdown` — include it only when browsing is on.
    if features.browsing {
        points.push(include_str!("turn/scaffold/web_fetch.md"));
    }
    // The transcript-link dotpoint teaches `convo.turn` — include it only when transcripts are on.
    points.extend(transcript.as_deref());
    points.extend([
        include_str!("turn/scaffold/particulars.md"),
        include_str!("turn/scaffold/dedup.md"),
        include_str!("turn/scaffold/look_before_acting.md"),
    ]);
    // The structured-relationship dotpoint teaches `links.create` — include it only when linking is
    // on.
    if features.linking {
        points.push(include_str!("turn/scaffold/structured_relationship.md"));
    }
    points.extend([
        include_str!("turn/scaffold/conflicts.md"),
        include_str!("turn/scaffold/correction.md"),
        include_str!("turn/scaffold/commit_honesty.md"),
        include_str!("turn/scaffold/visibility.md"),
        include_str!("turn/scaffold/volatility.md"),
        include_str!("turn/scaffold/turn_skip.md"),
    ]);

    let mut out = body_of(include_str!("turn/scaffold/preamble.md"));
    for point in points {
        out.push_str("\n\n- ");
        out.push_str(&render(point));
    }
    out
}

/// The recall point carries a linking-gated hub-walk clause inline: link-following is the `linking`
/// feature, so the clause is dropped when linking is off and the dotpoint never teaches a disabled
/// API.
fn recall_point(features: &InstanceFeatures) -> String {
    let hub = if features.linking {
        format!(" {}", body_of(include_str!("turn/scaffold/recall_hub.md")))
    } else {
        String::new()
    };
    include_str!("turn/scaffold/recall.md").replace("{{recall_hub}}", &hub)
}

/// Substitute the namespace-prefix placeholders from [`Namespace`], the one place the agent is taught
/// the prefixes, so the scaffold cannot drift from the handles the code mints and reads (the prefixes
/// carry their trailing slash).
fn render(raw: &str) -> String {
    body_of(raw)
        .replace("{{person}}", Namespace::Person.prefix())
        .replace("{{place}}", Namespace::Place.prefix())
        .replace("{{event}}", Namespace::Event.prefix())
        .replace("{{topic}}", Namespace::Topic.prefix())
        .replace("{{context}}", Namespace::Context.prefix())
        .replace("{{self}}", MemoryName::SELF)
}

/// Strip the trailing newline `include_str!` carries, so an embedded body matches the original
/// literal, which ended at its last character.
fn body_of(raw: &str) -> String {
    raw.trim_end_matches('\n').to_owned()
}
