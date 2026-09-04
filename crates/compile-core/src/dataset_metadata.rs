//! Per-dataset metadata sidecars for Forest Data Exchange.
//!
//! New ingest writes these next to each GFB3 file. Datasets that entered the
//! archive before that pipeline have no sidecar; `inspect_file` rebuilds one
//! from the table itself so they can be registered and compiled like new ones.

use crate::attributes::{detect_attributes, looks_like_gfb3};
use crate::country_lookup::suggest_from_coordinates;
use crate::geo::{BIOREGION_COLUMNS, COUNTRY_COLUMNS};
use crate::match_files::list_data_files;
use crate::raster_lookup::{LayerSources, RasterCatalog, RasterCatalogStatus, RasterSuggestions};
use crate::project_filter::{CONTINENT_COLUMNS, FOREST_TYPE_COLUMNS, YEAR_COLUMNS};
use crate::reader::{peek_column_uniques, peek_coordinate_pairs, peek_headers};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "forest-data-exchange.dataset_metadata";
pub const VERSION: u32 = 1;
const INSPECT_ROWS: usize = 200_000;

/// Website / compile-manifest attribute flags, in display order.
pub const ATTRIBUTE_KEYS: &[&str] = &[
    "tree_height",
    "agb",
    "wood_density",
    "crown_diameter",
    "mortality",
    "recruitment",
    "coordinates",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMetadata {
    pub schema: String,
    pub version: u32,
    pub generated_at: String,
    pub generated_by: String,
    pub source: SourceFile,
    #[serde(default)]
    pub contributor_email: Option<String>,
    #[serde(default)]
    pub contributor_name: Option<String>,
    /// People to contact and to seed publication author lists.
    /// Field names match Forest Data Exchange `publication_authors`.
    #[serde(default)]
    pub coauthors: Vec<Coauthor>,
    #[serde(default)]
    pub attributes: HashMap<String, bool>,
    #[serde(default)]
    pub geography: DatasetGeography,
    #[serde(default)]
    pub forest_types: Vec<String>,
    /// `column`, `raster`, or `manual`.
    #[serde(default)]
    pub forest_type_source: Option<String>,
    #[serde(default)]
    pub year_range: Option<YearSpan>,
    #[serde(default)]
    pub num_plots: Option<u64>,
    #[serde(default)]
    pub num_trees: Option<u64>,
    #[serde(default)]
    pub sampled_rows: Option<u64>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceFile {
    pub file_name: String,
    pub path: String,
    #[serde(default)]
    pub looks_like_gfb3: bool,
    #[serde(default)]
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatasetGeography {
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub continents: Vec<String>,
    #[serde(default)]
    pub ecoregions: Vec<String>,
    /// `column`, `coordinates`, or `manual`.
    #[serde(default)]
    pub country_source: Option<String>,
    /// `column`, `raster`, or `manual`.
    #[serde(default)]
    pub ecoregion_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Coauthor {
    pub author_name: String,
    #[serde(default)]
    pub author_email: Option<String>,
    /// `corresponding` or `co_author` — same vocabulary as the website.
    #[serde(default = "default_coauthor_role")]
    pub role: String,
    #[serde(default)]
    pub author_order: Option<u32>,
    #[serde(default)]
    pub affiliation: Option<String>,
}

fn default_coauthor_role() -> String {
    "co_author".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YearSpan {
    pub year_start: Option<i64>,
    pub year_end: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetadataInspect {
    pub metadata: DatasetMetadata,
    pub sidecar_path: String,
    pub sidecar_exists: bool,
    #[serde(default)]
    pub suggested_countries: Vec<String>,
    #[serde(default)]
    pub suggested_ecoregions: Vec<String>,
    #[serde(default)]
    pub suggested_forest_types: Vec<String>,
    #[serde(default)]
    pub authors_path: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct InspectBundle {
    pub items: Vec<MetadataInspect>,
    pub rasters: RasterCatalogStatus,
}

#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    pub overwrite: bool,
    pub contributor_email: Option<String>,
    pub contributor_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteReport {
    pub written: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

impl DatasetMetadata {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let m: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if m.schema != SCHEMA {
            return Err(format!(
                "unsupported schema '{}' (expected {SCHEMA})",
                m.schema
            ));
        }
        if m.version == 0 || m.version > VERSION {
            return Err(format!("unsupported dataset metadata version {}", m.version));
        }
        Ok(m)
    }

    pub fn to_pretty_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn continent_hint(&self) -> Option<&str> {
        if self.geography.continents.len() == 1 {
            Some(self.geography.continents[0].as_str())
        } else {
            None
        }
    }

    pub fn ecoregion_hint(&self) -> Option<&str> {
        if self.geography.ecoregions.len() == 1 {
            Some(self.geography.ecoregions[0].as_str())
        } else {
            None
        }
    }

    /// Keep the dataset owner on the author list so contact/publication
    /// exports do not drop them.
    pub fn ensure_owner_on_author_list(&mut self) {
        let name = self
            .contributor_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let email = self
            .contributor_email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if name.is_none() && email.is_none() {
            return;
        }
        let already = self.coauthors.iter().any(|c| {
            emails_match(c.author_email.as_deref(), email)
                || names_match(Some(c.author_name.as_str()), name)
        });
        if already {
            return;
        }
        self.coauthors.insert(
            0,
            Coauthor {
                author_name: name.unwrap_or("").to_string(),
                author_email: email.map(|s| s.to_string()),
                role: "corresponding".into(),
                author_order: Some(1),
                affiliation: None,
            },
        );
        renumber_authors(&mut self.coauthors);
    }
}

/// `{stem}.metadata.json` sitting next to a GFB3 table.
pub fn sidecar_path(data_path: &Path) -> PathBuf {
    sibling_with_suffix(data_path, ".metadata.json")
}

/// `{stem}_authors.json` sitting next to the original dataset.
pub fn authors_path(data_path: &Path) -> PathBuf {
    sibling_with_suffix(data_path, "_authors.json")
}

fn sibling_with_suffix(data_path: &Path, suffix: &str) -> PathBuf {
    let stem = data_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dataset");
    match data_path.parent() {
        Some(parent) => parent.join(format!("{stem}{suffix}")),
        None => PathBuf::from(format!("{stem}{suffix}")),
    }
}

pub fn load_alongside(data_path: &Path) -> Option<DatasetMetadata> {
    let path = sidecar_path(data_path);
    DatasetMetadata::from_path(&path).ok()
}

pub fn inspect_file(path: &Path) -> Result<MetadataInspect, String> {
    inspect_file_with(path, None)
}

pub fn inspect_file_with(
    path: &Path,
    catalog: Option<&RasterCatalog>,
) -> Result<MetadataInspect, String> {
    let (mut metadata, hints) = infer_from_file(path, catalog)?;
    let sidecar = sidecar_path(path);
    if let Ok(existing) = DatasetMetadata::from_path(&sidecar) {
        merge_existing_sidecar(&mut metadata, &existing);
    }
    Ok(MetadataInspect {
        sidecar_exists: sidecar.is_file(),
        sidecar_path: sidecar.display().to_string(),
        authors_path: authors_path(path).display().to_string(),
        suggested_countries: hints.suggested_countries,
        suggested_ecoregions: hints.suggested_ecoregions,
        suggested_forest_types: hints.suggested_forest_types,
        metadata,
    })
}

pub fn inspect_folder(folder: &Path, recursive: bool) -> Result<Vec<MetadataInspect>, String> {
    Ok(inspect_with_rasters(folder, recursive, &LayerSources::default())?.items)
}

pub fn inspect_with_rasters(
    folder: &Path,
    recursive: bool,
    sources: &LayerSources,
) -> Result<InspectBundle, String> {
    if !folder.is_dir() {
        return Err(format!("Not a folder: {}", folder.display()));
    }
    let catalog = RasterCatalog::load(sources, Some(folder));
    let rasters = catalog
        .as_ref()
        .map(|c| c.status())
        .unwrap_or_else(|| RasterCatalogStatus {
            message: "No map layer — pick a shapefile or GeoTIFF for ecoregion and/or forest type".into(),
            ..RasterCatalogStatus::default()
        });
    let files = list_data_files(folder, recursive)?;
    let mut items = Vec::new();
    for path in files {
        match inspect_file_with(&path, catalog.as_ref()) {
            Ok(item) => items.push(item),
            Err(e) => {
                let file_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                items.push(MetadataInspect {
                    sidecar_path: sidecar_path(&path).display().to_string(),
                    sidecar_exists: sidecar_path(&path).is_file(),
                    authors_path: authors_path(&path).display().to_string(),
                    suggested_countries: Vec::new(),
                    suggested_ecoregions: Vec::new(),
                    suggested_forest_types: Vec::new(),
                    metadata: DatasetMetadata {
                        schema: SCHEMA.into(),
                        version: VERSION,
                        generated_at: Utc::now().to_rfc3339(),
                        generated_by: generated_by(),
                        source: SourceFile {
                            file_name,
                            path: path.display().to_string(),
                            looks_like_gfb3: false,
                            columns: Vec::new(),
                        },
                        notes: vec![e],
                        ..empty_metadata_rest()
                    },
                });
            }
        }
    }
    Ok(InspectBundle { items, rasters })
}

pub fn write_sidecars(items: &[DatasetMetadata], opts: &WriteOptions) -> Result<WriteReport, String> {
    let mut report = WriteReport {
        written: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };
    for item in items {
        let source = PathBuf::from(&item.source.path);
        if source.as_os_str().is_empty() {
            report.errors.push(format!(
                "{}: missing source path",
                item.source.file_name
            ));
            continue;
        }
        let dest = sidecar_path(&source);
        if dest.is_file() && !opts.overwrite {
            report
                .skipped
                .push(format!("{} (sidecar already exists)", dest.display()));
            continue;
        }
        let mut meta = item.clone();
        if meta.contributor_email.as_deref().unwrap_or("").trim().is_empty() {
            if let Some(email) = opts.contributor_email.as_deref().map(str::trim) {
                if !email.is_empty() {
                    meta.contributor_email = Some(email.to_string());
                }
            }
        }
        if meta.contributor_name.as_deref().unwrap_or("").trim().is_empty() {
            if let Some(name) = opts.contributor_name.as_deref().map(str::trim) {
                if !name.is_empty() {
                    meta.contributor_name = Some(name.to_string());
                }
            }
        }
        sanitize_lists(&mut meta);
        meta.ensure_owner_on_author_list();
        renumber_authors(&mut meta.coauthors);
        refresh_notes(&mut meta);
        meta.generated_at = Utc::now().to_rfc3339();
        meta.generated_by = generated_by();
        match meta.to_pretty_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&dest, json) {
                    report.errors.push(format!("{}: {e}", dest.display()));
                } else {
                    report.written.push(dest.display().to_string());
                }
            }
            Err(e) => report.errors.push(format!("{}: {e}", item.source.file_name)),
        }
    }
    Ok(report)
}

struct InferHints {
    suggested_countries: Vec<String>,
    suggested_ecoregions: Vec<String>,
    suggested_forest_types: Vec<String>,
}

fn infer_from_file(
    path: &Path,
    catalog: Option<&RasterCatalog>,
) -> Result<(DatasetMetadata, InferHints), String> {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let columns = peek_headers(path).map_err(|e| e.to_string())?;
    let gfb3 = looks_like_gfb3(&columns);
    let detected = detect_attributes(&columns);
    let mut attributes = HashMap::new();
    for key in ATTRIBUTE_KEYS {
        attributes.insert(
            (*key).to_string(),
            detected.get(*key).copied().unwrap_or(false),
        );
    }

    let mut countries = peek_column_uniques(path, COUNTRY_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let mut continents = peek_column_uniques(path, CONTINENT_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let mut ecoregions = peek_column_uniques(path, BIOREGION_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let points = peek_coordinate_pairs(path, INSPECT_ROWS).unwrap_or_default();
    let (coord_countries, coord_continents) = suggest_from_coordinates(&points);
    let raster = catalog
        .map(|c| c.suggest(&points))
        .unwrap_or_else(RasterSuggestions::default);
    let mut country_source = if !countries.is_empty() {
        Some("column".into())
    } else {
        None
    };
    if countries.is_empty() && !coord_countries.is_empty() {
        countries = coord_countries.clone();
        country_source = Some("coordinates".into());
    }
    if continents.is_empty() && !coord_continents.is_empty() {
        continents = coord_continents;
    }
    let mut ecoregion_source = if !ecoregions.is_empty() {
        Some("column".into())
    } else {
        None
    };
    if ecoregions.is_empty() && !raster.ecoregions.is_empty() {
        ecoregions = raster.ecoregions.clone();
        ecoregion_source = raster
            .ecoregion_source
            .clone()
            .or_else(|| Some("geotiff".into()));
    }
    let mut forest_types =
        peek_column_uniques(path, FOREST_TYPE_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let mut forest_type_source = if !forest_types.is_empty() {
        Some("column".into())
    } else {
        None
    };
    if forest_types.is_empty() && !raster.forest_types.is_empty() {
        forest_types = raster.forest_types.clone();
        forest_type_source = raster
            .forest_type_source
            .clone()
            .or_else(|| Some("geotiff".into()));
    }
    let year_values = peek_column_uniques(path, YEAR_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let plots = peek_column_uniques(path, &["PlotID", "Plot_ID", "plotid"], INSPECT_ROWS)
        .unwrap_or_default();
    let trees = peek_column_uniques(path, &["TreeID", "Tree_ID", "treeid"], INSPECT_ROWS)
        .unwrap_or_default();

    let mut years: Vec<i64> = year_values.iter().filter_map(|y| parse_year(y)).collect();
    years.sort_unstable();
    let year_range = if years.is_empty() {
        None
    } else {
        Some(YearSpan {
            year_start: years.first().copied(),
            year_end: years.last().copied(),
        })
    };

    let sampled_rows = plots
        .len()
        .max(trees.len())
        .max(year_values.len())
        .max(countries.len()) as u64;

    let mut notes = Vec::new();
    if !gfb3 {
        notes.push("Does not look like a GFB3 tree table (few core columns found)".into());
    }
    if country_source.as_deref() == Some("coordinates") {
        notes.push(format!(
            "Countries suggested from plot coordinates (review near borders): {}",
            countries.join(", ")
        ));
    } else if countries.is_empty() && continents.is_empty() && ecoregions.is_empty() {
        if points.is_empty() {
            notes.push(
                "No Country column and no plot coordinates — add geography in Edit metadata"
                    .into(),
            );
        } else {
            notes.push(
                "Plot coordinates did not match a known country box — add country in Edit metadata"
                    .into(),
            );
        }
    }
    if is_map_layer_source(ecoregion_source.as_deref())
        || is_map_layer_source(forest_type_source.as_deref())
    {
        let gez = if raster.gez_labels.is_empty() {
            String::new()
        } else {
            format!(" ({})", raster.gez_labels.join(", "))
        };
        notes.push(format!(
            "Ecoregion / forest type suggested from map layer{gez} — review before writing"
        ));
    } else if forest_types.is_empty() {
        notes.push(
            "No ForestType column — choose forest types in Edit metadata, or pick a shapefile / GeoTIFF"
                .into(),
        );
    }
    if year_range.is_none() {
        notes.push("No YR / Year column — set the census window in Edit metadata".into());
    }
    notes.push(
        "Add contributor and coauthors in Edit metadata before registering on Forest Data Exchange."
            .into(),
    );

    Ok((DatasetMetadata {
        schema: SCHEMA.into(),
        version: VERSION,
        generated_at: Utc::now().to_rfc3339(),
        generated_by: generated_by(),
        source: SourceFile {
            file_name,
            path: path.display().to_string(),
            looks_like_gfb3: gfb3,
            columns,
        },
        contributor_email: None,
        contributor_name: None,
        coauthors: Vec::new(),
        attributes,
        geography: DatasetGeography {
            countries,
            continents,
            ecoregions,
            country_source,
            ecoregion_source,
        },
        forest_types,
        forest_type_source,
        year_range,
        num_plots: if plots.is_empty() {
            None
        } else {
            Some(plots.len() as u64)
        },
        num_trees: if trees.is_empty() {
            None
        } else {
            Some(trees.len() as u64)
        },
        sampled_rows: Some(sampled_rows),
        notes,
    }, InferHints {
        suggested_countries: coord_countries,
        suggested_ecoregions: raster.ecoregions,
        suggested_forest_types: raster.forest_types,
    }))
}

fn empty_metadata_rest() -> DatasetMetadata {
    DatasetMetadata {
        schema: SCHEMA.into(),
        version: VERSION,
        generated_at: Utc::now().to_rfc3339(),
        generated_by: generated_by(),
        source: SourceFile::default(),
        contributor_email: None,
        contributor_name: None,
        coauthors: Vec::new(),
        attributes: HashMap::new(),
        geography: DatasetGeography::default(),
        forest_types: Vec::new(),
        forest_type_source: None,
        year_range: None,
        num_plots: None,
        num_trees: None,
        sampled_rows: None,
        notes: Vec::new(),
    }
}

fn merge_existing_sidecar(into: &mut DatasetMetadata, existing: &DatasetMetadata) {
    if into.contributor_email.is_none() {
        into.contributor_email = existing.contributor_email.clone();
    }
    if into.contributor_name.is_none() {
        into.contributor_name = existing.contributor_name.clone();
    }
    if !existing.coauthors.is_empty() {
        into.coauthors = existing.coauthors.clone();
    }
    if !existing.geography.countries.is_empty() {
        into.geography.countries = existing.geography.countries.clone();
        into.geography.country_source = existing
            .geography
            .country_source
            .clone()
            .or_else(|| Some("manual".into()));
    }
    if !existing.geography.continents.is_empty() {
        into.geography.continents = existing.geography.continents.clone();
    }
    if !existing.geography.ecoregions.is_empty() {
        into.geography.ecoregions = existing.geography.ecoregions.clone();
        into.geography.ecoregion_source = existing
            .geography
            .ecoregion_source
            .clone()
            .or_else(|| Some("manual".into()));
    }
    if !existing.forest_types.is_empty() {
        into.forest_types = existing.forest_types.clone();
        into.forest_type_source = existing
            .forest_type_source
            .clone()
            .or_else(|| Some("manual".into()));
    }
    if existing.year_range.is_some() {
        into.year_range = existing.year_range.clone();
    }
    if !existing.attributes.is_empty() {
        into.attributes = existing.attributes.clone();
    }
}

fn sanitize_lists(meta: &mut DatasetMetadata) {
    let clean = |vals: &mut Vec<String>| {
        vals.retain(|s| !s.trim().is_empty());
        for s in vals.iter_mut() {
            *s = s.trim().to_string();
        }
        vals.sort();
        vals.dedup();
    };
    clean(&mut meta.geography.countries);
    clean(&mut meta.geography.continents);
    clean(&mut meta.geography.ecoregions);
    clean(&mut meta.forest_types);
}

fn has_contact(meta: &DatasetMetadata) -> bool {
    let named = meta
        .contributor_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    let emailed = meta
        .contributor_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    let authors = meta.coauthors.iter().any(|c| {
        !c.author_name.trim().is_empty()
            || c.author_email
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
    });
    named || emailed || authors
}

fn is_map_layer_source(source: Option<&str>) -> bool {
    matches!(source, Some("raster" | "geotiff" | "shapefile" | "combined"))
}

fn refresh_notes(meta: &mut DatasetMetadata) {
    let mut notes = Vec::new();
    if !meta.source.looks_like_gfb3 {
        notes.push("Does not look like a GFB3 tree table (few core columns found)".into());
    }
    if meta.geography.country_source.as_deref() == Some("coordinates") {
        notes.push(format!(
            "Countries suggested from plot coordinates (review near borders): {}",
            meta.geography.countries.join(", ")
        ));
    } else if meta.geography.countries.is_empty()
        && meta.geography.continents.is_empty()
        && meta.geography.ecoregions.is_empty()
    {
        notes.push(
            "No geography yet — add country, continent or bioregion in Edit metadata".into(),
        );
    }
    if is_map_layer_source(meta.geography.ecoregion_source.as_deref()) {
        notes.push(format!(
            "Ecoregions suggested from {}: {}",
            meta.geography.ecoregion_source.as_deref().unwrap_or("map layer"),
            meta.geography.ecoregions.join(", ")
        ));
    }
    if is_map_layer_source(meta.forest_type_source.as_deref()) {
        notes.push(format!(
            "Forest types suggested from {}: {}",
            meta.forest_type_source.as_deref().unwrap_or("map layer"),
            meta.forest_types.join(", ")
        ));
    } else if meta.forest_types.is_empty() {
        notes.push("No forest types yet — choose them in Edit metadata".into());
    }
    if meta.year_range.is_none() {
        notes.push("No census years yet — set the year window in Edit metadata".into());
    }
    if !has_contact(meta) {
        notes.push(
            "Add contributor and coauthors in Edit metadata before registering on Forest Data Exchange."
                .into(),
        );
    }
    meta.notes = notes;
}

pub fn write_author_sidecars(items: &[DatasetMetadata]) -> Result<WriteReport, String> {
    let mut report = WriteReport {
        written: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };
    for item in items {
        let source = PathBuf::from(&item.source.path);
        if source.as_os_str().is_empty() {
            report
                .errors
                .push(format!("{}: missing source path", item.source.file_name));
            continue;
        }
        let dest = authors_path(&source);
        match write_author_directory(std::slice::from_ref(item), &dest) {
            Ok(path) => report.written.push(path),
            Err(e) => report.errors.push(format!("{}: {e}", item.source.file_name)),
        }
    }
    Ok(report)
}

fn emails_match(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            let x = x.trim().to_lowercase();
            let y = y.trim().to_lowercase();
            !x.is_empty() && x == y
        }
        _ => false,
    }
}

fn names_match(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            let x = x.trim().to_lowercase();
            let y = y.trim().to_lowercase();
            !x.is_empty() && x == y
        }
        _ => false,
    }
}

fn renumber_authors(authors: &mut [Coauthor]) {
    for (i, author) in authors.iter_mut().enumerate() {
        author.author_order = Some((i + 1) as u32);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorDirectory {
    pub schema: String,
    pub version: u32,
    pub generated_at: String,
    pub generated_by: String,
    pub people: Vec<DirectoryPerson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryPerson {
    pub author_name: String,
    pub author_email: Option<String>,
    pub affiliation: Option<String>,
    pub roles: Vec<String>,
    pub datasets: Vec<String>,
}

/// Unique people across datasets, for contact lists and publication drafts.
pub fn build_author_directory(items: &[DatasetMetadata]) -> AuthorDirectory {
    #[derive(Default)]
    struct Acc {
        name: String,
        email: Option<String>,
        affiliation: Option<String>,
        roles: BTreeSet<String>,
        datasets: BTreeSet<String>,
    }

    let mut by_key: HashMap<String, Acc> = HashMap::new();
    for item in items {
        let dataset = item.source.file_name.clone();
        for author in &item.coauthors {
            let key = person_key(author.author_email.as_deref(), Some(&author.author_name));
            if key.is_empty() {
                continue;
            }
            let acc = by_key.entry(key).or_default();
            if acc.name.is_empty() && !author.author_name.trim().is_empty() {
                acc.name = author.author_name.trim().to_string();
            }
            if acc.email.is_none() {
                acc.email = author
                    .author_email
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
            }
            if acc.affiliation.is_none() {
                acc.affiliation = author
                    .affiliation
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
            }
            if !author.role.trim().is_empty() {
                acc.roles.insert(author.role.trim().to_string());
            }
            if !dataset.is_empty() {
                acc.datasets.insert(dataset.clone());
            }
        }
    }

    let mut people: Vec<DirectoryPerson> = by_key
        .into_values()
        .map(|acc| DirectoryPerson {
            author_name: acc.name,
            author_email: acc.email,
            affiliation: acc.affiliation,
            roles: acc.roles.into_iter().collect(),
            datasets: acc.datasets.into_iter().collect(),
        })
        .collect();
    people.sort_by(|a, b| {
        a.author_name
            .to_lowercase()
            .cmp(&b.author_name.to_lowercase())
            .then(
                a.author_email
                    .clone()
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&b.author_email.clone().unwrap_or_default().to_lowercase()),
            )
    });

    AuthorDirectory {
        schema: "forest-data-exchange.author_directory".into(),
        version: 1,
        generated_at: Utc::now().to_rfc3339(),
        generated_by: generated_by(),
        people,
    }
}

pub fn write_author_directory(items: &[DatasetMetadata], output: &Path) -> Result<String, String> {
    let dir = build_author_directory(items);
    let json = serde_json::to_string_pretty(&dir).map_err(|e| e.to_string())?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(output, json).map_err(|e| e.to_string())?;
    Ok(output.display().to_string())
}

fn person_key(email: Option<&str>, name: Option<&str>) -> String {
    if let Some(e) = email.map(str::trim).filter(|s| !s.is_empty()) {
        return format!("email:{}", e.to_lowercase());
    }
    name.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("name:{}", s.to_lowercase()))
        .unwrap_or_default()
}

fn generated_by() -> String {
    format!("forest-data-compiler {}", env!("CARGO_PKG_VERSION"))
}

fn parse_year(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<i64>() {
        return Some(n);
    }
    t.parse::<f64>().ok().map(|f| f.floor() as i64)
}
