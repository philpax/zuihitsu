//! The agent's runtime label vocabulary: typed lenses over the tag and relation names that live in
//! data (spec §Data model). The agent registers tags and relations at runtime, so these are not a
//! closed schema; rather, the build's meaningful labels are named variants code can match on, and
//! everything else falls to `Other`. Each serializes as its bare name, so the wire format is just
//! the string.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// A tag's name. Like [`RelationName`], the build's meaningful tags are named variants code can
/// match — `Confidential` drives the room-confidentiality marker (see spec §Visibility → marker) —
/// and everything else falls to `Other`. It serializes as its bare name, so the wire format is just
/// the string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagName {
    Confidential,
    Other(SmolStr),
}

impl TagName {
    /// Recognize a tag name, mapping a build-meaningful tag to its variant and anything else (an
    /// agent- or operator-created tag) to [`TagName::Other`]. Takes `&str` so a known variant is
    /// matched without allocating — only `Other` owns its `SmolStr`.
    pub fn new(name: &str) -> TagName {
        match name {
            "confidential" => TagName::Confidential,
            _ => TagName::Other(SmolStr::new(name)),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            TagName::Confidential => "confidential",
            TagName::Other(name) => name.as_str(),
        }
    }
}

impl Serialize for TagName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TagName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<TagName, D::Error> {
        let name = SmolStr::deserialize(deserializer)?;
        Ok(TagName::new(&name))
    }
}

impl std::str::FromStr for TagName {
    type Err = std::convert::Infallible;

    /// Recognize a tag name from a string, mapping a build-meaningful tag to its variant and anything
    /// else to [`TagName::Other`]. Never fails — the `Infallible` error communicates that every string
    /// is a valid tag name.
    fn from_str(name: &str) -> Result<TagName, Self::Err> {
        Ok(TagName::new(name))
    }
}

/// A link relation, by label. The relation registry lives in data (spec §Data model) and the agent
/// registers relations at runtime, so this is a typed lens over the names: the build's seed
/// relations are named variants that code can match (`SameAs` drives identity-class merging,
/// `ParticipatesIn` event attendance, `PartOf` membership or aboutness), and everything else —
/// including the inverse labels and every relation the agent coins for its own environment
/// (mentorship, venues, employment) — falls to `Other`. It serializes as its bare name, so the wire
/// format is just the string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationName {
    CreatedBy,
    OperatorOf,
    Knows,
    SameAs,
    ParticipatesIn,
    PartOf,
    /// The inverse label of [`RelationName::CreatedBy`].
    Created,
    /// The inverse label of [`RelationName::OperatorOf`].
    OperatedBy,
    /// The inverse label of [`RelationName::Knows`].
    KnownBy,
    /// The inverse label of [`RelationName::ParticipatesIn`].
    HasParticipant,
    /// The inverse label of [`RelationName::PartOf`].
    Contains,
    LocatedAt,
    /// The inverse label of [`RelationName::LocatedAt`].
    LocationOf,
    Other(SmolStr),
}

impl RelationName {
    /// Recognize a label, mapping a seed relation — or its inverse — to its variant and anything
    /// else (a runtime-registered relation) to [`RelationName::Other`]. [`RelationName::SameAs`] is
    /// its own inverse, so it has no separate variant. Takes `&str` so a known variant is matched
    /// without allocating — only `Other` owns its `SmolStr`.
    pub fn new(name: &str) -> RelationName {
        match name {
            "created_by" => RelationName::CreatedBy,
            "operator_of" => RelationName::OperatorOf,
            "knows" => RelationName::Knows,
            "same_as" => RelationName::SameAs,
            "participates_in" => RelationName::ParticipatesIn,
            "part_of" => RelationName::PartOf,
            "created" => RelationName::Created,
            "operated_by" => RelationName::OperatedBy,
            "known_by" => RelationName::KnownBy,
            "has_participant" => RelationName::HasParticipant,
            "contains" => RelationName::Contains,
            "located_at" => RelationName::LocatedAt,
            "location_of" => RelationName::LocationOf,
            _ => RelationName::Other(SmolStr::new(name)),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            RelationName::CreatedBy => "created_by",
            RelationName::OperatorOf => "operator_of",
            RelationName::Knows => "knows",
            RelationName::SameAs => "same_as",
            RelationName::ParticipatesIn => "participates_in",
            RelationName::PartOf => "part_of",
            RelationName::Created => "created",
            RelationName::OperatedBy => "operated_by",
            RelationName::KnownBy => "known_by",
            RelationName::HasParticipant => "has_participant",
            RelationName::Contains => "contains",
            RelationName::LocatedAt => "located_at",
            RelationName::LocationOf => "location_of",
            RelationName::Other(name) => name.as_str(),
        }
    }
}

impl Serialize for RelationName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelationName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<RelationName, D::Error> {
        let name = SmolStr::deserialize(deserializer)?;
        Ok(RelationName::new(&name))
    }
}

impl std::str::FromStr for RelationName {
    type Err = std::convert::Infallible;

    /// Recognize a relation label from a string, mapping a seed relation — or its inverse — to its
    /// variant and anything else to [`RelationName::Other`]. Never fails — the `Infallible` error
    /// communicates that every string is a valid relation label.
    fn from_str(name: &str) -> Result<RelationName, Self::Err> {
        Ok(RelationName::new(name))
    }
}
