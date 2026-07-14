//! The trait and types for extracting settings field metadata at compile time via the
//! `settings-metadata-derive` proc-macro. The export-types binary collects from all settings
//! types and writes `settings-metadata.ts`.

/// One field's metadata: its dotted path and the `///` doc comment as the description.
pub struct SettingsField {
    pub path: String,
    pub description: String,
}

/// A trait implemented by the `SettingsMetadata` derive for each settings struct. Returns the
/// struct's fields with their dotted paths and doc-comment descriptions.
pub trait SettingsMetadata {
    fn fields() -> Vec<SettingsField>;
}
