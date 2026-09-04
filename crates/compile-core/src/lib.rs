//! Forest Data Exchange — Forest Data Compiler core.
//!
//! Reads `forest-data-exchange.compile_manifest` JSON, matches local GFB3 files
//! against registered suitable datasets / attribute rules, and vertically
//! concatenates selected tables into one compiled product.

pub mod attributes;
pub mod compile;
pub mod country_lookup;
pub mod dataset_metadata;
pub mod geo;
pub mod raster_lookup;
pub mod manifest;
pub mod match_files;
pub mod project_filter;
pub mod reader;

#[cfg(test)]
mod tests_integration;

pub use attributes::{detect_attributes, ATTRIBUTE_SYNONYMS};
pub use compile::{compile_files, CompileFormat, CompileOptions, CompileReport};
pub use dataset_metadata::{
    authors_path, inspect_file, inspect_folder, inspect_with_rasters, sidecar_path,
    write_author_directory, write_author_sidecars, write_sidecars, Coauthor,
    DatasetMetadata, InspectBundle, MetadataInspect, WriteOptions, WriteReport,
    SCHEMA as DATASET_METADATA_SCHEMA,
};
pub use geo::{GeoFilter, GeoMode, BIOREGIONS, CONTINENTS, FOREST_TYPES};
pub use manifest::{CompileManifest, ManifestScope, RegisteredDataset, LEGACY_SCHEMA, SCHEMA};
pub use match_files::{match_folder, CandidateFile, MatchOptions, MatchReason, ScanBundle};
pub use raster_lookup::{
    LayerSources, RasterCatalog, RasterCatalogStatus, raster_search_dirs,
};
pub use reader::read_file;
