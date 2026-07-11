// Test imports shared by both test modules — the types they reference from the instance module.
#[cfg(test)]
use {
    super::OpenSession,
    crate::{Instance, store::Store, vector::VectorIndex},
};

#[cfg(test)]
mod designation;
#[cfg(test)]
mod embedding_swap;
#[cfg(test)]
mod observability;
#[cfg(test)]
mod priority;
#[cfg(test)]
mod self_edit;
#[cfg(test)]
mod unmerge;
