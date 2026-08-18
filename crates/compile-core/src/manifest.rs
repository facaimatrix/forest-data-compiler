use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SCHEMA: &str = "forest-data-exchange.compile_manifest";

/// Schema id used before the InterNodes → Forest Data Exchange rename.
pub const LEGACY_SCHEMA: &str = "internodes.compile_manifest";

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported schema '{0}' (expected {SCHEMA})")]
    BadSchema(String),
    #[error("unsupported manifest version {0}")]
    BadVersion(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileManifest {
    pub schema: String,
    pub version: u32,
    pub generated_at: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub project: ProjectInfo,
    pub requirements: Requirements,
    #[serde(default)]
    pub matching_rules: MatchingRules,
    #[serde(default)]
    pub registered_datasets: RegisteredDatasets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub pi_email: Option<String>,
    #[serde(default)]
    pub pi_name: Option<String>,
    #[serde(default)]
    pub census_frequency: Option<String>,
    #[serde(default)]
    pub board_deadline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Requirements {
    #[serde(default)]
    pub guaranteed_fields: Vec<String>,
    #[serde(default)]
    pub mandatory_attributes: Vec<AttributeReq>,
    #[serde(default)]
    pub ideal_attributes: Vec<AttributeReq>,
    #[serde(default)]
    pub mandatory_map_products: Vec<AttributeReq>,
    #[serde(default)]
    pub ideal_map_products: Vec<AttributeReq>,
    #[serde(default)]
    pub geography: Geography,
    #[serde(default)]
    pub forest_types: Vec<String>,
    #[serde(default)]
    pub year_range: Option<YearRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeReq {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub dataset_flag: Option<String>,
}

impl AttributeReq {
    pub fn display(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.key.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Geography {
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub continents: Vec<String>,
    #[serde(default)]
    pub ecoregions: Vec<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub extent_shapefile_path: Option<String>,
    #[serde(default)]
    pub extent_shapefile_name: Option<String>,
}

fn default_scope() -> String {
    "global".into()
}

/// Census window. `year_end` may be a number, a numeric string, or "present".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YearRange {
    #[serde(default)]
    pub year_start: Option<YearBound>,
    #[serde(default)]
    pub year_end: Option<YearBound>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YearBound {
    Num(i64),
    Text(String),
}

impl YearBound {
    /// Numeric year, or None for open-ended bounds such as "present".
    pub fn as_year(&self) -> Option<i64> {
        match self {
            Self::Num(n) => Some(*n),
            Self::Text(s) => s.trim().parse::<i64>().ok(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Num(n) => n.to_string(),
            Self::Text(s) => s.trim().to_string(),
        }
    }
}

impl YearRange {
    pub fn start_year(&self) -> Option<i64> {
        self.year_start.as_ref().and_then(|b| b.as_year())
    }

    pub fn end_year(&self) -> Option<i64> {
        self.year_end.as_ref().and_then(|b| b.as_year())
    }

    pub fn label(&self) -> String {
        let start = self
            .year_start
            .as_ref()
            .map(|b| b.label())
            .unwrap_or_else(|| "any".into());
        let end = self
            .year_end
            .as_ref()
            .map(|b| b.label())
            .unwrap_or_else(|| "present".into());
        format!("{start} – {end}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchingRules {
    #[serde(default = "default_true")]
    pub dataset_must_have_all_mandatory_attributes: bool,
    #[serde(default)]
    pub geography: Option<String>,
    #[serde(default)]
    pub year_range: Option<String>,
    #[serde(default)]
    pub forest_types: Option<String>,
    #[serde(default = "default_true")]
    pub ideal_attributes_are_optional: bool,
    #[serde(default = "default_true")]
    pub map_products_are_outputs: bool,
}

impl Default for MatchingRules {
    fn default() -> Self {
        Self {
            dataset_must_have_all_mandatory_attributes: true,
            geography: None,
            year_range: None,
            forest_types: None,
            ideal_attributes_are_optional: true,
            map_products_are_outputs: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegisteredDatasets {
    #[serde(default)]
    pub suitable: Vec<RegisteredDataset>,
    #[serde(default)]
    pub joined_owner_emails: Vec<String>,
    #[serde(default)]
    pub pending_owner_emails: Vec<String>,
    #[serde(default)]
    pub compatible_plots: Option<f64>,
    #[serde(default)]
    pub join_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredDataset {
    pub id: String,
    pub file_name: String,
    #[serde(default)]
    pub contributor_email: Option<String>,
    #[serde(default)]
    pub contributor_name: Option<String>,
    #[serde(default)]
    pub continent: Option<String>,
    #[serde(default)]
    pub ecoregion: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub num_plots: Option<f64>,
    #[serde(default)]
    pub attributes: std::collections::HashMap<String, bool>,
}

/// Geographic scope of a project, normalized from `requirements.geography.scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestScope {
    Global,
    Continental,
    Ecoregion,
    Countries,
    Shapefile,
}

impl CompileManifest {
    pub fn from_path(path: &std::path::Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, ManifestError> {
        let m: Self = serde_json::from_str(text)?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != SCHEMA && self.schema != LEGACY_SCHEMA {
            return Err(ManifestError::BadSchema(self.schema.clone()));
        }
        if self.version == 0 || self.version > 1 {
            return Err(ManifestError::BadVersion(self.version));
        }
        Ok(())
    }

    pub fn scope(&self) -> ManifestScope {
        match self.requirements.geography.scope.trim().to_lowercase().as_str() {
            "continental" | "continent" | "continents" => ManifestScope::Continental,
            "ecoregion" | "ecoregions" | "bioregion" | "bioregions" => ManifestScope::Ecoregion,
            "countries" | "country" => ManifestScope::Countries,
            "shapefile" | "extent" | "custom" => ManifestScope::Shapefile,
            _ => ManifestScope::Global,
        }
    }

    pub fn mandatory_keys(&self) -> Vec<String> {
        self.requirements
            .mandatory_attributes
            .iter()
            .map(|a| a.key.clone())
            .collect()
    }

    pub fn ideal_keys(&self) -> Vec<String> {
        self.requirements
            .ideal_attributes
            .iter()
            .map(|a| a.key.clone())
            .collect()
    }

    /// Selected forest types, with the "All" wildcard removed.
    /// Empty means no forest-type restriction.
    pub fn forest_type_filter(&self) -> Vec<String> {
        self.requirements
            .forest_types
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
            .collect()
    }

    /// Inclusive census-year bounds; None on either side means open-ended.
    pub fn year_bounds(&self) -> (Option<i64>, Option<i64>) {
        match &self.requirements.year_range {
            Some(r) => (r.start_year(), r.end_year()),
            None => (None, None),
        }
    }

    pub fn joined_emails_set(&self) -> std::collections::HashSet<String> {
        self.registered_datasets
            .joined_owner_emails
            .iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect()
    }
}
