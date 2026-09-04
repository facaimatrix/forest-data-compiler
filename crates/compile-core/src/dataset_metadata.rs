//! Per-dataset metadata sidecars for Forest Data Exchange.
//!
//! New ingest writes these next to each GFB3 file. Datasets that entered the
//! archive before that pipeline have no sidecar; `inspect_file` rebuilds one
//! from the table itself so they can be registered and compiled like new ones.

use crate::attributes::{detect_attributes, looks_like_gfb3};
use crate::geo::{BIOREGION_COLUMNS, COUNTRY_COLUMNS};
use crate::match_files::list_data_files;
use crate::project_filter::{CONTINENT_COLUMNS, FOREST_TYPE_COLUMNS, YEAR_COLUMNS};
use crate::reader::{peek_column_uniques, peek_headers};
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
    let stem = data_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dataset");
    match data_path.parent() {
        Some(parent) => parent.join(format!("{stem}.metadata.json")),
        None => PathBuf::from(format!("{stem}.metadata.json")),
    }
}

pub fn load_alongside(data_path: &Path) -> Option<DatasetMetadata> {
    let path = sidecar_path(data_path);
    DatasetMetadata::from_path(&path).ok()
}

pub fn inspect_file(path: &Path) -> Result<MetadataInspect, String> {
    let mut metadata = infer_from_file(path)?;
    let sidecar = sidecar_path(path);
    if let Ok(existing) = DatasetMetadata::from_path(&sidecar) {
        merge_people_from_existing(&mut metadata, &existing);
    }
    Ok(MetadataInspect {
        sidecar_exists: sidecar.is_file(),
        sidecar_path: sidecar.display().to_string(),
        metadata,
    })
}

pub fn inspect_folder(folder: &Path, recursive: bool) -> Result<Vec<MetadataInspect>, String> {
    if !folder.is_dir() {
        return Err(format!("Not a folder: {}", folder.display()));
    }
    let files = list_data_files(folder, recursive)?;
    let mut out = Vec::new();
    for path in files {
        match inspect_file(&path) {
            Ok(item) => out.push(item),
            Err(e) => {
                let file_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                out.push(MetadataInspect {
                    sidecar_path: sidecar_path(&path).display().to_string(),
                    sidecar_exists: sidecar_path(&path).is_file(),
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
    Ok(out)
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
        if let Some(email) = opts.contributor_email.as_deref().map(str::trim) {
            if !email.is_empty() {
                meta.contributor_email = Some(email.to_string());
            }
        }
        if let Some(name) = opts.contributor_name.as_deref().map(str::trim) {
            if !name.is_empty() {
                meta.contributor_name = Some(name.to_string());
            }
        }
        meta.ensure_owner_on_author_list();
        renumber_authors(&mut meta.coauthors);
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

fn infer_from_file(path: &Path) -> Result<DatasetMetadata, String> {
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

    let countries = peek_column_uniques(path, COUNTRY_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let continents = peek_column_uniques(path, CONTINENT_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let ecoregions = peek_column_uniques(path, BIOREGION_COLUMNS, INSPECT_ROWS).unwrap_or_default();
    let forest_types =
        peek_column_uniques(path, FOREST_TYPE_COLUMNS, INSPECT_ROWS).unwrap_or_default();
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
    if countries.is_empty() && continents.is_empty() && ecoregions.is_empty() {
        notes.push(
            "No Country / Continent / Bioregion column — geography must be filled in by hand"
                .into(),
        );
    }
    if forest_types.is_empty() {
        notes.push("No ForestType column — forest types unknown".into());
    }
    if year_range.is_none() {
        notes.push("No YR / Year column — census window unknown".into());
    }
    notes.push(
        "Contributor name and email are not in the table; add them before registering on Forest Data Exchange."
            .into(),
    );

    Ok(DatasetMetadata {
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
        },
        forest_types,
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
    })
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
        year_range: None,
        num_plots: None,
        num_trees: None,
        sampled_rows: None,
        notes: Vec::new(),
    }
}

fn merge_people_from_existing(into: &mut DatasetMetadata, existing: &DatasetMetadata) {
    if into.contributor_email.is_none() {
        into.contributor_email = existing.contributor_email.clone();
    }
    if into.contributor_name.is_none() {
        into.contributor_name = existing.contributor_name.clone();
    }
    if !existing.coauthors.is_empty() {
        into.coauthors = existing.coauthors.clone();
    }
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
