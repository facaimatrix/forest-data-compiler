//! Geographic filter modes for offline compilation.

use serde::{Deserialize, Serialize};

/// Forest Data Exchange bioregion vocabulary (same list as the website's ECOREGIONS).
pub const BIOREGIONS: &[&str] = &[
    "Tropical moist broadleaf forests",
    "Tropical dry broadleaf forests",
    "Tropical coniferous forests",
    "Temperate broadleaf & mixed forests",
    "Temperate coniferous forests",
    "Boreal forests/taiga",
    "Mediterranean forests",
    "Mangroves",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GeoMode {
    #[default]
    Global,
    Bioregions,
    ByCountry,
}

impl GeoMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "bioregions" | "bioregion" | "ecoregion" | "ecoregions" => Self::Bioregions,
            "by_country" | "country" | "countries" => Self::ByCountry,
            _ => Self::Global,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Bioregions => "bioregions",
            Self::ByCountry => "by_country",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeoFilter {
    pub mode: GeoMode,
    /// Selected bioregion names when mode == Bioregions.
    #[serde(default)]
    pub bioregions: Vec<String>,
    /// Selected country names when mode == ByCountry.
    #[serde(default)]
    pub countries: Vec<String>,
}

impl GeoFilter {
    pub fn allows_file(
        &self,
        registered_ecoregion: Option<&str>,
        file_countries: &[String],
        file_bioregions: &[String],
    ) -> bool {
        match self.mode {
            GeoMode::Global => true,
            GeoMode::Bioregions => {
                if self.bioregions.is_empty() {
                    return true;
                }
                let mut hits = file_bioregions.to_vec();
                if let Some(e) = registered_ecoregion {
                    hits.push(e.to_string());
                }
                hits.iter().any(|h| {
                    self.bioregions
                        .iter()
                        .any(|b| b.eq_ignore_ascii_case(h.trim()))
                })
            }
            GeoMode::ByCountry => {
                if self.countries.is_empty() {
                    return true;
                }
                file_countries.iter().any(|c| {
                    self.countries
                        .iter()
                        .any(|sel| sel.eq_ignore_ascii_case(c.trim()))
                })
            }
        }
    }

    pub fn row_filter_column(&self) -> Option<(&'static str, &[String])> {
        match self.mode {
            GeoMode::Global => None,
            GeoMode::Bioregions if !self.bioregions.is_empty() => {
                Some(("__bioregion__", &self.bioregions))
            }
            GeoMode::ByCountry if !self.countries.is_empty() => {
                Some(("Country", &self.countries))
            }
            _ => None,
        }
    }
}

/// Column names that may hold bioregion / ecoregion labels in a GFB3 table.
pub const BIOREGION_COLUMNS: &[&str] = &[
    "Bioregion",
    "BioRegion",
    "Ecoregion",
    "EcoRegion",
    "Biome",
];

pub const COUNTRY_COLUMNS: &[&str] = &["Country", "country", "COUNTRY", "Nation"];

pub const CONTINENTS: &[&str] = &[
    "Africa",
    "Asia",
    "Europe",
    "North America",
    "South America",
    "Oceania",
];

pub const FOREST_TYPES: &[&str] = &[
    "Tropical",
    "Subtropical",
    "Temperate",
    "Boreal",
    "Coniferous",
    "Deciduous",
    "Mixed",
    "Mediterranean",
    "Mangrove",
    "Montane",
    "Dry forest",
    "Plantation",
];
