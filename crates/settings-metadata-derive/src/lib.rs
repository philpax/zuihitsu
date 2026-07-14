//! A proc-macro derive that extracts field names and `///` doc comments from settings structs,
//! producing a `SettingsMetadata` trait impl the export-types binary collects from.
//!
//! Usage:
//! ```ignore
//! #[derive(SettingsMetadata)]
//! #[settings_metadata(parent = "compaction")]
//! pub struct CompactionSettings { ... }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields, Lit, Meta, parse_macro_input};

/// Derive `SettingsMetadata` for a struct, extracting each field's name and `///` doc comment.
/// The `#[settings_metadata(parent = "...")]` attribute sets the dotted path prefix.
#[proc_macro_derive(SettingsMetadata, attributes(settings_metadata))]
pub fn derive_settings_metadata(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    // Extract the `parent` from `#[settings_metadata(parent = "...")]`.
    let parent = input
        .attrs
        .iter()
        .find_map(|attr| {
            if !attr.path().is_ident("settings_metadata") {
                return None;
            }
            let nested = attr.parse_args::<Meta>().ok()?;
            if let Meta::NameValue(nv) = nested
                && nv.path.is_ident("parent")
                && let syn::Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) = nv.value
            {
                return Some(s.value());
            }
            None
        })
        .unwrap_or_default();

    // Extract field names and doc comments.
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => panic!("SettingsMetadata only supports structs with named fields"),
        },
        _ => panic!("SettingsMetadata only supports structs"),
    };

    let field_entries: Vec<proc_macro2::TokenStream> = fields
        .iter()
        .map(|field| {
            let name = field.ident.as_ref().expect("named field");
            let name_str = name.to_string();

            // Collect `///` doc comments, which appear as `#[doc = "..."]` attributes.
            let docs: Vec<String> = field
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("doc"))
                .filter_map(|attr| {
                    let Meta::NameValue(ref nv) = attr.meta else {
                        return None;
                    };
                    if !nv.path.is_ident("doc") {
                        return None;
                    }
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(ref s),
                        ..
                    }) = nv.value
                    {
                        return Some(s.value());
                    }
                    None
                })
                .collect();

            // Collapse multi-line doc comments the same way the Node script did: strip leading
            // whitespace and ` * ` markers, join with spaces.
            let description = docs
                .iter()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");

            quote! {
                crate::settings_metadata::SettingsField {
                    path: format!("{}.{}", #parent, #name_str),
                    description: #description.to_owned(),
                }
            }
        })
        .collect();

    let expanded = quote! {
        impl crate::settings_metadata::SettingsMetadata for #struct_name {
            fn fields() -> Vec<crate::settings_metadata::SettingsField> {
                vec![#(#field_entries),*]
            }
        }
    };

    expanded.into()
}
