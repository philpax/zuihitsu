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
    /// agent- or operator-created tag) to [`TagName::Other`].
    pub fn new(name: impl Into<SmolStr>) -> TagName {
        let name = name.into();
        match name.as_str() {
            "confidential" => TagName::Confidential,
            _ => TagName::Other(name),
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
        Ok(TagName::new(SmolStr::deserialize(deserializer)?))
    }
}

/// A link relation, by label. The relation registry lives in data (spec §Data model) and the agent
/// registers relations at runtime, so this is a typed lens over the names: the build's seed
/// relations are named variants that code can match (`SameAs` drives identity-class merging,
/// `ActiveIn` the compaction carryover), and everything else — including the inverse labels — falls
/// to `Other`. It serializes as its bare name, so the wire format is just the string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationName {
    CreatedBy,
    OperatorOf,
    Knows,
    SameAs,
    ActiveIn,
    /// The inverse label of [`RelationName::CreatedBy`].
    Created,
    /// The inverse label of [`RelationName::OperatorOf`].
    Operates,
    /// The inverse label of [`RelationName::Knows`].
    KnownBy,
    /// The inverse label of [`RelationName::ActiveIn`].
    HasActive,
    Other(SmolStr),
}

impl RelationName {
    /// Recognize a label, mapping a seed relation — or its inverse — to its variant and anything
    /// else (a runtime-registered relation) to [`RelationName::Other`]. [`RelationName::SameAs`] is
    /// its own inverse, so it has no separate variant.
    pub fn new(name: impl Into<SmolStr>) -> RelationName {
        let name = name.into();
        match name.as_str() {
            "created_by" => RelationName::CreatedBy,
            "operator_of" => RelationName::OperatorOf,
            "knows" => RelationName::Knows,
            "same_as" => RelationName::SameAs,
            "active_in" => RelationName::ActiveIn,
            "created" => RelationName::Created,
            "operates" => RelationName::Operates,
            "known_by" => RelationName::KnownBy,
            "has_active" => RelationName::HasActive,
            _ => RelationName::Other(name),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            RelationName::CreatedBy => "created_by",
            RelationName::OperatorOf => "operator_of",
            RelationName::Knows => "knows",
            RelationName::SameAs => "same_as",
            RelationName::ActiveIn => "active_in",
            RelationName::Created => "created",
            RelationName::Operates => "operates",
            RelationName::KnownBy => "known_by",
            RelationName::HasActive => "has_active",
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
        Ok(RelationName::new(SmolStr::deserialize(deserializer)?))
    }
}
