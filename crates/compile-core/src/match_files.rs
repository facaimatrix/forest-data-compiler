use crate::attributes::{detect_attributes, ideal_score, looks_like_gfb3};
use crate::dataset_metadata::{load_alongside, DatasetMetadata};
use crate::geo::{
    GeoFilter, GeoMode, BIOREGIONS, BIOREGION_COLUMNS, COUNTRY_COLUMNS,
};
use crate::manifest::{CompileManifest, ManifestScope, RegisteredDataset};
use crate::project_filter::FOREST_TYPE_COLUMNS;
use crate::reader::{is_supported_extension, peek_column_uniques, peek_headers};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct MatchOptions {
    /// Only keep registered datasets whose owner is in joined_owner_emails.
    pub joined_owners_only: bool,
    /// Also accept unregistered local files that look like GFB3 and pass
    /// mandatory attribute column checks.
    pub include_unregistered: bool,
    /// Recurse into subfolders.
    pub recursive: bool,
    /// Geographic filter applied on top of the manifest.
    pub geo: GeoFilter,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchReason {
    RegisteredName,
    SidecarMetadata,
    ColumnAttributes,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateFile {
    pub path: String,
    pub file_name: String,
    pub selected: bool,
    pub reason: MatchReason,
    pub looks_like_gfb3: bool,
    pub registered: Option<RegisteredHit>,
    pub detected_attributes: HashMap<String, bool>,
    pub missing_mandatory: Vec<String>,
    pub ideal_hits: usize,
    pub ideal_total: usize,
    pub geography_ok: bool,
    pub forest_type_ok: bool,
    pub countries_found: Vec<String>,
    pub bioregions_found: Vec<String>,
    pub forest_types_found: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// Outcome of checking one file against the project's geography.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeoCheck {
    Ok,
    Fail,
    /// The file carries no label we can compare against.
    Unverified,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisteredHit {
    pub id: String,
    pub file_name: String,
    pub contributor_email: Option<String>,
    pub contributor_name: Option<String>,
    pub continent: Option<String>,
    pub ecoregion: Option<String>,
    pub num_plots: Option<f64>,
    pub owner_joined: bool,
    pub attributes: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanBundle {
    pub candidates: Vec<CandidateFile>,
    pub discovered_countries: Vec<String>,
    pub discovered_forest_types: Vec<String>,
    pub bioregion_options: Vec<String>,
}

pub fn match_folder(
    folder: &Path,
    manifest: &CompileManifest,
    opts: &MatchOptions,
) -> Result<ScanBundle, String> {
    if !folder.is_dir() {
        return Err(format!("Not a folder: {}", folder.display()));
    }

    let files = list_data_files(folder, opts.recursive)?;
    let joined = manifest.joined_emails_set();
    let mandatory = manifest.mandatory_keys();
    let ideal = manifest.ideal_keys();
    let suitable = &manifest.registered_datasets.suitable;

    let mut out = Vec::new();
    let mut all_countries: BTreeSet<String> = BTreeSet::new();
    let mut all_forest_types: BTreeSet<String> = BTreeSet::new();
    let forest_type_filter = manifest.forest_type_filter();
    let enforce_mandatory = manifest
        .matching_rules
        .dataset_must_have_all_mandatory_attributes;

    for path in files {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let sidecar = load_alongside(&path);
        let from_manifest = find_registered(&file_name, suitable);
        let registered = from_manifest
            .map(|ds| registered_hit_from_dataset(ds, &joined))
            .or_else(|| sidecar.as_ref().map(|m| registered_hit_from_sidecar(m, &joined)));

        if registered.is_none() && !opts.include_unregistered {
            continue;
        }

        if let Some(ref hit) = registered {
            if opts.joined_owners_only && !hit.owner_joined {
                let sidecar_unknown = from_manifest.is_none()
                    && sidecar
                        .as_ref()
                        .and_then(|m| m.contributor_email.as_deref())
                        .map(|e| e.trim().is_empty())
                        .unwrap_or(true);
                if !sidecar_unknown {
                    continue;
                }
            }
        }

        let mut warnings = Vec::new();
        let (columns, read_err) = match peek_headers(&path) {
            Ok(cols) => (cols, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };

        let detected = if columns.is_empty() {
            HashMap::new()
        } else {
            detect_attributes(&columns)
        };
        let gfb3 = !columns.is_empty() && looks_like_gfb3(&columns);

        // Lightweight geo peek (first 2k rows) — only when needed
        let mut countries_found = Vec::new();
        let mut bioregions_found = Vec::new();
        let mut forest_types_found = Vec::new();
        if read_err.is_none() {
            if opts.geo.mode == GeoMode::ByCountry
                || manifest.scope() == ManifestScope::Countries
                || columns.iter().any(|h| {
                    COUNTRY_COLUMNS
                        .iter()
                        .any(|c| h.eq_ignore_ascii_case(c))
                })
            {
                countries_found =
                    peek_column_uniques(&path, COUNTRY_COLUMNS, 2000).unwrap_or_default();
                for c in &countries_found {
                    all_countries.insert(c.clone());
                }
            }
            if opts.geo.mode == GeoMode::Bioregions
                || columns.iter().any(|h| {
                    BIOREGION_COLUMNS
                        .iter()
                        .any(|c| h.eq_ignore_ascii_case(c))
                })
            {
                bioregions_found =
                    peek_column_uniques(&path, BIOREGION_COLUMNS, 2000).unwrap_or_default();
            }
            if let Some(ref hit) = registered {
                if let Some(e) = &hit.ecoregion {
                    if !e.trim().is_empty() {
                        bioregions_found.push(e.clone());
                    }
                }
            }
            bioregions_found.sort();
            bioregions_found.dedup();

            if !forest_type_filter.is_empty()
                || columns.iter().any(|h| {
                    FOREST_TYPE_COLUMNS
                        .iter()
                        .any(|c| h.eq_ignore_ascii_case(c))
                })
            {
                forest_types_found =
                    peek_column_uniques(&path, FOREST_TYPE_COLUMNS, 2000).unwrap_or_default();
                for f in &forest_types_found {
                    all_forest_types.insert(f.clone());
                }
            }
        }

        if let Some(ref meta) = sidecar {
            merge_unique(&mut countries_found, &meta.geography.countries);
            merge_unique(&mut bioregions_found, &meta.geography.ecoregions);
            merge_unique(&mut forest_types_found, &meta.forest_types);
            for c in &countries_found {
                all_countries.insert(c.clone());
            }
            for f in &forest_types_found {
                all_forest_types.insert(f.clone());
            }
        }

        let attr_source: HashMap<String, bool> = if let Some(ref hit) = registered {
            if hit.attributes.is_empty() {
                detected.clone()
            } else {
                hit.attributes.clone()
            }
        } else {
            detected.clone()
        };

        let missing_mandatory: Vec<String> = mandatory
            .iter()
            .filter(|k| !attr_source.get(k.as_str()).copied().unwrap_or(false))
            .cloned()
            .collect();

        // Project geography from the manifest (global / continental / bioregion / countries)
        let manifest_geo_ok = match manifest_geography_check(
            manifest,
            registered.as_ref().and_then(|r| r.continent.as_deref()),
            registered.as_ref().and_then(|r| r.ecoregion.as_deref()),
            &countries_found,
        ) {
            GeoCheck::Ok => true,
            GeoCheck::Fail => false,
            GeoCheck::Unverified => {
                warnings.push("Project geography could not be verified from this file".into());
                true
            }
        };

        // Desktop geo filter (Global / Bioregions / By country)
        let filter_geo_ok = opts.geo.allows_file(
            registered.as_ref().and_then(|r| r.ecoregion.as_deref()),
            &countries_found,
            &bioregions_found,
        );

        let geography_ok = manifest_geo_ok && filter_geo_ok;

        if !manifest_geo_ok {
            warnings.push("Outside project geography in the manifest".into());
        }
        if !filter_geo_ok {
            match opts.geo.mode {
                GeoMode::Bioregions => {
                    warnings.push("Does not match selected bioregions".into());
                }
                GeoMode::ByCountry => {
                    warnings.push("Does not match selected countries".into());
                }
                GeoMode::Global => {}
            }
        }
        // Project forest types: only a hard fail when the file states its types
        // and none of them are wanted.
        let forest_type_ok = if forest_type_filter.is_empty() || forest_types_found.is_empty() {
            true
        } else {
            forest_types_found.iter().any(|f| {
                forest_type_filter
                    .iter()
                    .any(|want| want.eq_ignore_ascii_case(f.trim()))
            })
        };
        if !forest_type_ok {
            warnings.push(format!(
                "No rows of the required forest type ({})",
                forest_type_filter.join(", ")
            ));
        }

        if !missing_mandatory.is_empty() {
            let suffix = if enforce_mandatory {
                ""
            } else {
                " (not enforced by this project)"
            };
            warnings.push(format!(
                "Missing mandatory attributes{suffix}: {}",
                missing_mandatory.join(", ")
            ));
        }
        if registered.is_none() && !gfb3 {
            warnings.push("Does not look like a GFB3 tree table".into());
        }

        let reason = if from_manifest.is_some() {
            MatchReason::RegisteredName
        } else if sidecar.is_some() {
            MatchReason::SidecarMetadata
        } else {
            MatchReason::ColumnAttributes
        };

        let mandatory_ok = missing_mandatory.is_empty() || !enforce_mandatory;
        let eligible = mandatory_ok
            && geography_ok
            && forest_type_ok
            && read_err.is_none()
            && (registered.is_some() || (opts.include_unregistered && gfb3));

        if registered.is_none() && (!opts.include_unregistered || !gfb3) {
            continue;
        }

        // When a geo filter is active, drop files that fail it from the list
        // unless they still have a registered match (show with warning, unselected).
        if !filter_geo_ok && registered.is_none() {
            continue;
        }

        out.push(CandidateFile {
            path: path.display().to_string(),
            file_name,
            selected: eligible,
            reason,
            looks_like_gfb3: gfb3,
            registered,
            detected_attributes: detected,
            missing_mandatory,
            ideal_hits: ideal_score(&attr_source, &ideal),
            ideal_total: ideal.len(),
            geography_ok,
            forest_type_ok,
            countries_found,
            bioregions_found,
            forest_types_found,
            warnings,
            error: read_err,
        });
    }

    out.sort_by(|a, b| {
        let aj = a
            .registered
            .as_ref()
            .map(|r| r.owner_joined)
            .unwrap_or(false);
        let bj = b
            .registered
            .as_ref()
            .map(|r| r.owner_joined)
            .unwrap_or(false);
        bj.cmp(&aj)
            .then(b.ideal_hits.cmp(&a.ideal_hits))
            .then(a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()))
    });

    let mut bioregion_options: Vec<String> = BIOREGIONS.iter().map(|s| (*s).to_string()).collect();
    for c in &out {
        for b in &c.bioregions_found {
            if !bioregion_options.iter().any(|x| x.eq_ignore_ascii_case(b)) {
                bioregion_options.push(b.clone());
            }
        }
    }

    Ok(ScanBundle {
        candidates: out,
        discovered_countries: all_countries.into_iter().collect(),
        discovered_forest_types: all_forest_types.into_iter().collect(),
        bioregion_options,
    })
}

pub fn list_data_files(folder: &Path, recursive: bool) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if recursive {
        for entry in walkdir(folder)? {
            if entry.is_file() && is_supported_extension(&entry) {
                out.push(entry);
            }
        }
    } else {
        let rd = std::fs::read_dir(folder).map_err(|e| e.to_string())?;
        for entry in rd {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_file() && is_supported_extension(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn walkdir(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
        for entry in rd {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn registered_hit_from_dataset(
    ds: &RegisteredDataset,
    joined: &std::collections::HashSet<String>,
) -> RegisteredHit {
    let email = ds
        .contributor_email
        .as_deref()
        .map(|e| e.trim().to_lowercase())
        .unwrap_or_default();
    let owner_joined = !email.is_empty() && joined.contains(&email);
    RegisteredHit {
        id: ds.id.clone(),
        file_name: ds.file_name.clone(),
        contributor_email: ds.contributor_email.clone(),
        contributor_name: ds.contributor_name.clone(),
        continent: ds.continent.clone(),
        ecoregion: ds.ecoregion.clone(),
        num_plots: ds.num_plots,
        owner_joined,
        attributes: ds.attributes.clone(),
    }
}

fn registered_hit_from_sidecar(
    meta: &DatasetMetadata,
    joined: &std::collections::HashSet<String>,
) -> RegisteredHit {
    let email = meta
        .contributor_email
        .as_deref()
        .map(|e| e.trim().to_lowercase())
        .unwrap_or_default();
    let owner_joined = !email.is_empty() && joined.contains(&email);
    RegisteredHit {
        id: format!("sidecar:{}", meta.source.file_name),
        file_name: meta.source.file_name.clone(),
        contributor_email: meta.contributor_email.clone(),
        contributor_name: meta.contributor_name.clone(),
        continent: meta.continent_hint().map(|s| s.to_string()),
        ecoregion: meta.ecoregion_hint().map(|s| s.to_string()),
        num_plots: meta.num_plots.map(|n| n as f64),
        owner_joined,
        attributes: meta.attributes.clone(),
    }
}

fn merge_unique(target: &mut Vec<String>, extra: &[String]) {
    for item in extra {
        let t = item.trim();
        if t.is_empty() {
            continue;
        }
        if !target.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            target.push(t.to_string());
        }
    }
}

fn find_registered<'a>(
    local_name: &str,
    suitable: &'a [RegisteredDataset],
) -> Option<&'a RegisteredDataset> {
    let local_norm = normalize_name(local_name);
    let local_stem = stem_norm(local_name);

    suitable.iter().find(|ds| {
        let reg = normalize_name(&ds.file_name);
        let reg_stem = stem_norm(&ds.file_name);
        reg == local_norm
            || reg_stem == local_stem
            || reg.contains(&local_stem)
            || local_norm.contains(&reg_stem)
    })
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn stem_norm(name: &str) -> String {
    let path = Path::new(name);
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .trim()
        .to_lowercase()
}

fn manifest_geography_check(
    manifest: &CompileManifest,
    continent: Option<&str>,
    ecoregion: Option<&str>,
    countries_found: &[String],
) -> GeoCheck {
    let geo = &manifest.requirements.geography;
    match manifest.scope() {
        ManifestScope::Global => GeoCheck::Ok,
        ManifestScope::Continental => {
            if geo.continents.is_empty() {
                return GeoCheck::Ok;
            }
            match continent {
                None => GeoCheck::Unverified,
                Some(c) => bool_check(geo.continents.iter().any(|x| x.eq_ignore_ascii_case(c))),
            }
        }
        ManifestScope::Ecoregion => {
            if geo.ecoregions.is_empty() {
                return GeoCheck::Ok;
            }
            match ecoregion {
                None => GeoCheck::Unverified,
                Some(e) => bool_check(geo.ecoregions.iter().any(|x| x.eq_ignore_ascii_case(e))),
            }
        }
        ManifestScope::Countries => {
            if geo.countries.is_empty() {
                return GeoCheck::Ok;
            }
            if countries_found.is_empty() {
                return GeoCheck::Unverified;
            }
            bool_check(countries_found.iter().any(|c| {
                geo.countries
                    .iter()
                    .any(|want| want.eq_ignore_ascii_case(c.trim()))
            }))
        }
        // Shapefile extents need GIS work the compiler does not do yet.
        ManifestScope::Shapefile => GeoCheck::Unverified,
    }
}

fn bool_check(ok: bool) -> GeoCheck {
    if ok {
        GeoCheck::Ok
    } else {
        GeoCheck::Fail
    }
}
