//! Forest Data Exchange — Forest Data Compiler core.
//!
//! Reads `forest-data-exchange.compile_manifest` JSON, matches local GFB3 files
//! against registered suitable datasets / attribute rules, and vertically
//! concatenates selected tables into one compiled product.

pub mod attributes;
pub mod compile;
pub mod geo;
pub mod manifest;
pub mod match_files;
pub mod project_filter;
pub mod reader;

#[cfg(test)]
mod tests_integration;

pub use attributes::{detect_attributes, ATTRIBUTE_SYNONYMS};
pub use compile::{compile_files, CompileFormat, CompileOptions, CompileReport};
pub use geo::{GeoFilter, GeoMode, BIOREGIONS};
pub use manifest::{CompileManifest, ManifestScope, RegisteredDataset, LEGACY_SCHEMA, SCHEMA};
pub use match_files::{match_folder, CandidateFile, MatchOptions, MatchReason, ScanBundle};
pub use reader::read_file;
