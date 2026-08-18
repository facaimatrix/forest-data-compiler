//! Detect Forest Data Exchange optional attributes from GFB3 column headers.

use std::collections::{HashMap, HashSet};

/// Attribute key → accepted column-name synonyms (case-insensitive).
pub const ATTRIBUTE_SYNONYMS: &[(&str, &[&str])] = &[
    (
        "tree_height",
        &["TreeH", "TreeHeight", "Tree_Height", "Height", "H", "Ht"],
    ),
    (
        "agb",
        &["AGB", "Agb", "Biomass", "AGB_Mg", "AbovegroundBiomass"],
    ),
    (
        "wood_density",
        &[
            "WD",
            "WoodDensity",
            "Wood_Density",
            "Density",
            "WoodDens",
            "rho",
        ],
    ),
    (
        "crown_diameter",
        &[
            "CrownD",
            "CrownDiameter",
            "Crown_Diameter",
            "CD",
            "CrownWidth",
        ],
    ),
    (
        "mortality",
        &["Mortality", "Mort", "Dead", "Death"],
    ),
    (
        "recruitment",
        &["Recruitment", "Recruit", "Ingrowth"],
    ),
    (
        "coordinates",
        &[
            "TreeX",
            "TreeY",
            "X",
            "Y",
            "TreeLat",
            "TreeLon",
            "TreeLatitude",
            "TreeLongitude",
            "StemX",
            "StemY",
        ],
    ),
];

/// Core GFB3 columns we expect in a tree-level file.
pub const GFB3_CORE_HINTS: &[&str] = &[
    "PlotID", "TreeID", "DBH", "YR", "Status", "Species", "Latitude", "Longitude", "PA",
];

pub fn detect_attributes(columns: &[String]) -> HashMap<String, bool> {
    let lower: HashSet<String> = columns.iter().map(|c| normalize(c)).collect();
    let mut out = HashMap::new();
    for (key, synonyms) in ATTRIBUTE_SYNONYMS {
        let hit = synonyms.iter().any(|s| lower.contains(&normalize(s)));
        // Mortality / recruitment can also be inferred from Status presence
        // only when explicitly tagged; we do not auto-infer from Status alone.
        out.insert((*key).to_string(), hit);
    }
    out
}

pub fn looks_like_gfb3(columns: &[String]) -> bool {
    let lower: HashSet<String> = columns.iter().map(|c| normalize(c)).collect();
    let hits = GFB3_CORE_HINTS
        .iter()
        .filter(|h| lower.contains(&normalize(h)))
        .count();
    hits >= 4
}

pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn ideal_score(detected: &HashMap<String, bool>, ideal_keys: &[String]) -> usize {
    ideal_keys
        .iter()
        .filter(|k| detected.get(k.as_str()).copied().unwrap_or(false))
        .count()
}
