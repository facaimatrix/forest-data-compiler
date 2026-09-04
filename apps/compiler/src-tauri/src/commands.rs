use compile_core::compile::{
    compile_files, default_output_name, CompileFormat, CompileOptions, CompileReport,
};
use compile_core::dataset_metadata::{
    inspect_with_rasters, write_author_sidecars, write_sidecars, DatasetMetadata, InspectBundle,
    WriteOptions, WriteReport,
};
use compile_core::geo::{GeoFilter, GeoMode, BIOREGIONS};
use compile_core::manifest::{AttributeReq, CompileManifest, ManifestScope};
use compile_core::match_files::{match_folder, CandidateFile, MatchOptions};
use compile_core::raster_lookup::LayerSources;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::command;

#[derive(Debug, Serialize)]
pub struct ManifestSummary {
    pub path: String,
    pub project_id: String,
    pub project_title: String,
    pub status: Option<String>,
    pub pi_name: Option<String>,
    pub pi_email: Option<String>,
    pub geography_scope: String,
    pub continents: Vec<String>,
    pub ecoregions: Vec<String>,
    pub countries: Vec<String>,
    pub extent_shapefile_name: Option<String>,
    pub mandatory: Vec<AttrLabel>,
    pub ideal: Vec<AttrLabel>,
    pub map_products: Vec<AttrLabel>,
    pub forest_types: Vec<String>,
    pub year_range_label: Option<String>,
    pub enforces_mandatory_attributes: bool,
    pub suitable_count: usize,
    pub joined_owners: usize,
    pub pending_owners: usize,
    pub join_percent: Option<f64>,
    pub compatible_plots: Option<f64>,
    pub notes: Vec<String>,
    pub bioregion_options: Vec<String>,
    /// Requirements the compiler cannot enforce yet, shown as a warning.
    pub unenforced: Vec<String>,
    pub raw: CompileManifest,
}

#[derive(Debug, Serialize)]
pub struct AttrLabel {
    pub key: String,
    pub label: String,
}

impl From<&AttributeReq> for AttrLabel {
    fn from(a: &AttributeReq) -> Self {
        Self {
            key: a.key.clone(),
            label: a.display(),
        }
    }
}

#[command]
pub async fn load_manifest(path: String) -> Result<ManifestSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = PathBuf::from(&path);
        let m = CompileManifest::from_path(&p).map_err(|e| e.to_string())?;
        let mut bioregion_options: Vec<String> =
            BIOREGIONS.iter().map(|s| (*s).to_string()).collect();
        for e in &m.requirements.geography.ecoregions {
            if !bioregion_options.iter().any(|x| x.eq_ignore_ascii_case(e)) {
                bioregion_options.push(e.clone());
            }
        }

        let mut unenforced = Vec::new();
        if m.scope() == ManifestScope::Shapefile {
            unenforced.push(format!(
                "Project extent shapefile ({}) — pick a local copy under Geographic filter → Reference shapefile to keep plots whose Latitude/Longitude fall inside it.",
                m.requirements
                    .geography
                    .extent_shapefile_name
                    .clone()
                    .unwrap_or_else(|| "unnamed".into())
            ));
        }
        if !m.requirements.mandatory_map_products.is_empty() {
            unenforced.push(format!(
                "Map products are project outputs, not produced here: {}",
                m.requirements
                    .mandatory_map_products
                    .iter()
                    .map(|p| p.display())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(ManifestSummary {
            path,
            project_id: m.project.id.clone(),
            project_title: m.project.title.clone(),
            status: m.project.status.clone(),
            pi_name: m.project.pi_name.clone(),
            pi_email: m.project.pi_email.clone(),
            geography_scope: m.requirements.geography.scope.clone(),
            continents: m.requirements.geography.continents.clone(),
            ecoregions: m.requirements.geography.ecoregions.clone(),
            countries: m.requirements.geography.countries.clone(),
            extent_shapefile_name: m.requirements.geography.extent_shapefile_name.clone(),
            mandatory: m
                .requirements
                .mandatory_attributes
                .iter()
                .map(AttrLabel::from)
                .collect(),
            ideal: m
                .requirements
                .ideal_attributes
                .iter()
                .map(AttrLabel::from)
                .collect(),
            map_products: m
                .requirements
                .mandatory_map_products
                .iter()
                .chain(m.requirements.ideal_map_products.iter())
                .map(AttrLabel::from)
                .collect(),
            forest_types: m.requirements.forest_types.clone(),
            year_range_label: m.requirements.year_range.as_ref().map(|r| r.label()),
            enforces_mandatory_attributes: m
                .matching_rules
                .dataset_must_have_all_mandatory_attributes,
            suitable_count: m.registered_datasets.suitable.len(),
            joined_owners: m.registered_datasets.joined_owner_emails.len(),
            pending_owners: m.registered_datasets.pending_owner_emails.len(),
            join_percent: m.registered_datasets.join_percent,
            compatible_plots: m.registered_datasets.compatible_plots,
            notes: m.notes.clone(),
            bioregion_options,
            unenforced,
            raw: m,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
pub struct ScanInput {
    pub folder: String,
    pub manifest: CompileManifest,
    pub joined_owners_only: bool,
    pub include_unregistered: bool,
    pub recursive: bool,
    #[serde(default)]
    pub geo_mode: String,
    #[serde(default)]
    pub bioregions: Vec<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub shapefile_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub candidates: Vec<CandidateFile>,
    pub selected_count: usize,
    pub total_found: usize,
    pub discovered_countries: Vec<String>,
    pub discovered_forest_types: Vec<String>,
    pub bioregion_options: Vec<String>,
}

#[command]
pub async fn scan_folder(input: ScanInput) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let folder = PathBuf::from(&input.folder);
        let opts = MatchOptions {
            joined_owners_only: input.joined_owners_only,
            include_unregistered: input.include_unregistered,
            recursive: input.recursive,
            geo: GeoFilter {
                mode: GeoMode::parse(&input.geo_mode),
                bioregions: input.bioregions,
                countries: input.countries,
                shapefile_path: input.shapefile_path,
            },
        };
        let bundle = match_folder(&folder, &input.manifest, &opts)?;
        let selected_count = bundle.candidates.iter().filter(|c| c.selected).count();
        let total_found = bundle.candidates.len();
        Ok(ScanResult {
            candidates: bundle.candidates,
            selected_count,
            total_found,
            discovered_countries: bundle.discovered_countries,
            discovered_forest_types: bundle.discovered_forest_types,
            bioregion_options: bundle.bioregion_options,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
pub struct CompileInput {
    pub paths: Vec<String>,
    pub output_path: String,
    pub manifest: CompileManifest,
    /// csv | xlsx | parquet | zip | auto
    pub format: String,
    pub add_source_column: bool,
    #[serde(default)]
    pub geo_mode: String,
    #[serde(default)]
    pub bioregions: Vec<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub shapefile_path: Option<String>,
}

#[command]
pub async fn compile_selection(input: CompileInput) -> Result<CompileReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths: Vec<PathBuf> = input.paths.iter().map(PathBuf::from).collect();
        let output = PathBuf::from(&input.output_path);
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let format = parse_format(&input.format)?;
        let opts = CompileOptions {
            format,
            add_source_column: input.add_source_column,
            geo: GeoFilter {
                mode: GeoMode::parse(&input.geo_mode),
                bioregions: input.bioregions,
                countries: input.countries,
                shapefile_path: input.shapefile_path,
            },
        };
        compile_files(&paths, &output, &input.manifest, &opts)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[command]
pub async fn suggest_output_name(
    manifest: CompileManifest,
    format: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let f = parse_format(&format)?;
        Ok(default_output_name(&manifest, f))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
pub struct InspectMetadataInput {
    pub folder: String,
    pub recursive: bool,
    #[serde(default)]
    pub raster_folder: Option<String>,
    #[serde(default)]
    pub ecoregion_layer: Option<String>,
    #[serde(default)]
    pub forest_type_layer: Option<String>,
}

#[command]
pub async fn inspect_folder_metadata(
    input: InspectMetadataInput,
) -> Result<InspectBundle, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let opt_path = |s: Option<String>| {
            s.filter(|p| !p.trim().is_empty()).map(PathBuf::from)
        };
        inspect_with_rasters(
            &PathBuf::from(&input.folder),
            input.recursive,
            &LayerSources {
                folder: opt_path(input.raster_folder),
                ecoregion: opt_path(input.ecoregion_layer),
                forest_type: opt_path(input.forest_type_layer),
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
pub struct WriteMetadataInput {
    pub items: Vec<DatasetMetadata>,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub contributor_email: Option<String>,
    #[serde(default)]
    pub contributor_name: Option<String>,
}

#[command]
pub async fn write_dataset_metadata(input: WriteMetadataInput) -> Result<WriteReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        write_sidecars(
            &input.items,
            &WriteOptions {
                overwrite: input.overwrite,
                contributor_email: input.contributor_email,
                contributor_name: input.contributor_name,
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
pub struct AuthorDirectoryInput {
    pub items: Vec<DatasetMetadata>,
}

#[command]
pub async fn export_author_directory(input: AuthorDirectoryInput) -> Result<WriteReport, String> {
    tauri::async_runtime::spawn_blocking(move || write_author_sidecars(&input.items))
        .await
        .map_err(|e| e.to_string())?
}

fn parse_format(s: &str) -> Result<CompileFormat, String> {
    match s.trim().to_lowercase().as_str() {
        "csv" => Ok(CompileFormat::Csv),
        "xlsx" | "xls" => Ok(CompileFormat::Xlsx),
        "parquet" => Ok(CompileFormat::Parquet),
        "zip" | "zip_originals" => Ok(CompileFormat::ZipOriginals),
        "auto" | "" => Ok(CompileFormat::Auto),
        other => Err(format!("Unknown format '{other}'")),
    }
}
