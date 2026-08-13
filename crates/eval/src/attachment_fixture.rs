//! The attachment fixtures a scenario shares into a conversation: the bytes and media type behind
//! each [`AttachmentFixture`]. A step names a fixture; the executor resolves it here, stores the
//! bytes in the run's blob store, and builds the [`Attachment`](zuihitsu::Attachment) record the
//! platform delivers — the eval's stand-in for the connector upload a real platform performs.
//!
//! Two files are served. A plain-text note is the only place its specific details appear, so a reply
//! that reproduces one is structural proof the file was read rather than guessed. A small PNG — a
//! large solid yellow circle centred on a solid deep-blue ground — is unmistakable and easily named,
//! so a vision model's description of it is assessable.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use zuihitsu::ids::BlobHash;
use zuihitsu_frontend_types::PackageBlob;

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

/// Every fixture's bytes, as the package's blob catalogue — what a viewer resolves a run's attachment
/// addresses against when no agent stands behind it.
///
/// The catalogue is the whole fixture set rather than the subset a filtered run happens to share.
/// [`AttachmentFixture`] is a closed enum and [`fixture_bytes`] is the only path bytes reach a run's
/// blob store by, so the set is complete by construction, and it is complete from the first frame a
/// live watcher receives rather than filling in as runs finish. Two small files cost less than the
/// bookkeeping of tracking which ones a run touched.
///
/// A fixture over [`MAX_CATALOGUE_BLOB_BYTES`] is left out, and the viewer falls back to announcing it
/// by name, type, and size — the same degradation as an attachment whose bytes are gone.
pub fn catalogue() -> Vec<PackageBlob> {
    ALL_FIXTURES
        .iter()
        .map(|fixture| fixture_bytes(*fixture))
        .filter(|fixture| fixture.bytes.len() <= MAX_CATALOGUE_BLOB_BYTES)
        .map(|fixture| PackageBlob {
            hash: BlobHash::of(fixture.bytes),
            mime: fixture.mime.to_owned(),
            base64: STANDARD.encode(fixture.bytes),
        })
        .collect()
}

/// Every fixture a scenario can share. Listed rather than derived, so adding a variant to
/// [`AttachmentFixture`] without adding it here fails the exhaustiveness test below.
const ALL_FIXTURES: &[AttachmentFixture] =
    &[AttachmentFixture::VenueNote, AttachmentFixture::CoverDraft];

/// The largest fixture carried in a package. A package is read as a file and diffed as JSON, and
/// base64 costs a third again on top of the bytes; a fixture bigger than this belongs in the run for
/// the agent to read, not in every package for a reader to scroll past.
const MAX_CATALOGUE_BLOB_BYTES: usize = 4 * 1024 * 1024;

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

    use super::{
        ALL_FIXTURES, STANDARD, VENUE_NOTE, VENUE_NOTE_DOOR_CODE, catalogue, fixture_bytes,
    };
    use crate::step::AttachmentFixture;
    use base64::Engine as _;
    use zuihitsu::{AttachmentKind, ids::BlobHash};

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

    #[test]
    fn the_catalogue_carries_every_fixture_under_the_address_a_run_stores_it_at() {
        // The addresses must be the ones the recorded attachments name, or a viewer resolving a run's
        // attachment against the catalogue finds nothing.
        let catalogue = catalogue();
        for fixture in [AttachmentFixture::VenueNote, AttachmentFixture::CoverDraft] {
            let bytes = fixture_bytes(fixture);
            let entry = catalogue
                .iter()
                .find(|blob| blob.hash == BlobHash::of(bytes.bytes))
                .unwrap_or_else(|| panic!("the catalogue carries {fixture:?}"));
            assert_eq!(entry.mime, bytes.mime);
            assert_eq!(STANDARD.decode(&entry.base64).unwrap(), bytes.bytes);
        }
        assert_eq!(catalogue.len(), 2, "no fixture is carried twice");
    }

    #[test]
    fn every_fixture_variant_is_listed_in_the_catalogue_source() {
        // A new variant must be added to `ALL_FIXTURES`; this match is what fails to compile if it is
        // not, and the count check is what fails if the list is edited without the match.
        let listed = ALL_FIXTURES.len();
        for fixture in ALL_FIXTURES {
            match fixture {
                AttachmentFixture::VenueNote | AttachmentFixture::CoverDraft => {}
            }
        }
        assert_eq!(listed, 2);
    }
}
