//! The attachment fixtures a scenario shares into a conversation: the bytes and media type behind
//! each [`AttachmentFixture`]. A step names a fixture; the executor resolves it here, stores the
//! bytes in the run's blob store, and builds the [`Attachment`](zuihitsu::Attachment) record the
//! platform delivers — the eval's stand-in for the connector upload a real platform performs.
//!
//! Two files are served. A plain-text note is the only place its specific details appear, so a reply
//! that reproduces one is structural proof the file was read rather than guessed. A small PNG — a
//! large solid yellow circle centred on a solid deep-blue ground — is unmistakable and easily named,
//! so a vision model's description of it is assessable.

use crate::step::AttachmentFixture;

/// One fixture's stored form: the media type it is uploaded under, and its bytes.
pub struct FixtureBytes {
    pub mime: &'static str,
    pub bytes: &'static [u8],
}

/// The bytes and media type a fixture is delivered as.
pub fn fixture_bytes(fixture: AttachmentFixture) -> FixtureBytes {
    match fixture {
        AttachmentFixture::VenueNote => FixtureBytes {
            mime: "text/plain; charset=utf-8",
            bytes: VENUE_NOTE.as_bytes(),
        },
        AttachmentFixture::CoverDraft => FixtureBytes {
            mime: "image/png",
            bytes: COVER_DRAFT,
        },
    }
}

/// The roller-door code stated in the venue note and nowhere else — the fact a text-attachment
/// oracle checks a reply for, held here beside the file it comes from.
pub const VENUE_NOTE_DOOR_CODE: &str = "7742";

/// Notes typed up during a call with a venue coordinator: hire hours, capacities, the loading dock
/// and its door code, and the house rules. Short enough to inline whole under the attachment text
/// cap, so the whole note reaches the agent.
const VENUE_NOTE: &str = include_str!("fixtures/venue_note.txt");

/// A 320×320 PNG mockup: a large solid yellow disc centred on a solid deep-blue ground. Deliberately
/// a single named shape on a single named ground, so what the model saw is either right or wrong
/// rather than a matter of interpretation.
const COVER_DRAFT: &[u8] = include_bytes!("fixtures/cover_draft.png");

#[cfg(test)]
mod tests {
    //! The fixtures' contract with the oracles that read them: the note states the code an oracle
    //! looks for, and each fixture is delivered under the media type its scenario's behavior depends
    //! on.

    use super::{VENUE_NOTE, VENUE_NOTE_DOOR_CODE, fixture_bytes};
    use crate::step::AttachmentFixture;
    use zuihitsu::AttachmentKind;

    #[test]
    fn the_venue_note_states_the_code_its_oracle_looks_for() {
        assert!(VENUE_NOTE.contains(VENUE_NOTE_DOOR_CODE));
    }

    #[test]
    fn each_fixture_classifies_as_the_kind_its_scenario_depends_on() {
        let note = fixture_bytes(AttachmentFixture::VenueNote);
        assert_eq!(AttachmentKind::of_mime(note.mime), AttachmentKind::Text);
        let cover = fixture_bytes(AttachmentFixture::CoverDraft);
        assert_eq!(AttachmentKind::of_mime(cover.mime), AttachmentKind::Image);
        // The PNG signature, so a corrupted or replaced fixture fails here rather than at run time.
        assert!(cover.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
