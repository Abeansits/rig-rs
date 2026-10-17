// Derived from openrig packages/daemon/src/domain/context-packs/manifest-parser.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Context-pack manifests, addresses, and ref tokens.

mod atoms;
mod error;
mod files;
mod load;
mod markdown;
mod parse;
mod profiles;
mod safety;
mod source_ref;
mod types;

pub use error::{ManifestCode, ManifestError};
pub use load::load_yaml;
pub use markdown::{
    AddressError, AddressFinding, MarkdownSection, ParsedAddress, parse_address, parse_sections,
    resolve_address, slugify_header, validate_addressability,
};
pub use parse::parse_manifest;
pub use safety::{assert_safe_pack_ref, is_safe_pack_ref, is_safe_segment, is_safe_version};
pub use source_ref::{
    ParsedSourceRef, SourceKind, SourceRefError, is_seat_recap_address, parse_source_ref,
    source_kind_for_address,
};
pub use types::{
    Atom, AtomRuntime, FILE_SUFFIXES, PackFile, PackManifest, Priority, Probe, Profile,
    ProfilePhase, ProfileRuntime, ProfileSource, Purpose, Region, Situation, TAXONOMY_TEACHING,
    Taxonomy,
};
