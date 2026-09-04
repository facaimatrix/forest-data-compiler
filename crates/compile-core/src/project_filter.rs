//! Row-level filters driven by the project manifest: geography, forest type and
//! census-year window. These enforce `matching_rules` at compile time so the
//! compiled product cannot contain rows the project did not ask for.

use crate::geo::{BIOREGION_COLUMNS, COUNTRY_COLUMNS};
use crate::manifest::{CompileManifest, ManifestScope};
use crate::raster_lookup::ExtentMap;
use crate::reader::{parse_coord, LAT_COLUMNS, LON_COLUMNS};
use polars::prelude::*;

pub const CONTINENT_COLUMNS: &[&str] = &["Continent"];
pub const FOREST_TYPE_COLUMNS: &[&str] =
    &["ForestType", "Forest_Type", "Forest type", "ForestTypes", "Biome"];
pub const YEAR_COLUMNS: &[&str] = &["YR", "Year", "Yr", "YEAR", "Census_Year"];

/// Apply every manifest-driven row filter, returning the filtered frame plus
/// human-readable notes for the compile report.
pub fn apply_manifest_filters(
    df: DataFrame,
    manifest: &CompileManifest,
) -> Result<(DataFrame, Vec<String>), String> {
    let mut df = df;
    let mut notes = Vec::new();
    let geo = &manifest.requirements.geography;

    let geo_target: Option<(&[&str], &[String], &str)> = match manifest.scope() {
        ManifestScope::Countries if !geo.countries.is_empty() => {
            Some((COUNTRY_COLUMNS, &geo.countries, "country"))
        }
        ManifestScope::Continental if !geo.continents.is_empty() => {
            Some((CONTINENT_COLUMNS, &geo.continents, "continent"))
        }
        ManifestScope::Ecoregion if !geo.ecoregions.is_empty() => {
            Some((BIOREGION_COLUMNS, &geo.ecoregions, "bioregion"))
        }
        _ => None,
    };

    if let Some((columns, allowed, label)) = geo_target {
        let before = df.height();
        match filter_rows_by_values(df.clone(), columns, allowed)? {
            Some(filtered) => {
                notes.push(format!(
                    "Project {label} filter kept {} of {before} rows",
                    filtered.height()
                ));
                df = filtered;
            }
            None => notes.push(format!(
                "No {label} column found; project {label} filter could not be applied"
            )),
        }
    }

    let forest_types = manifest.forest_type_filter();
    if !forest_types.is_empty() {
        let before = df.height();
        match filter_rows_by_values(df.clone(), FOREST_TYPE_COLUMNS, &forest_types)? {
            Some(filtered) => {
                notes.push(format!(
                    "Forest-type filter ({}) kept {} of {before} rows",
                    forest_types.join(", "),
                    filtered.height()
                ));
                df = filtered;
            }
            None => notes.push(
                "No ForestType column found; project forest-type filter could not be applied"
                    .into(),
            ),
        }
    }

    let (start, end) = manifest.year_bounds();
    if start.is_some() || end.is_some() {
        let before = df.height();
        match filter_rows_by_year(df.clone(), start, end)? {
            Some(filtered) => {
                notes.push(format!(
                    "Year filter ({}–{}) kept {} of {before} rows",
                    start.map(|y| y.to_string()).unwrap_or_else(|| "any".into()),
                    end.map(|y| y.to_string()).unwrap_or_else(|| "present".into()),
                    filtered.height()
                ));
                df = filtered;
            }
            None => notes
                .push("No year column found; project year filter could not be applied".into()),
        }
    }

    Ok((df, notes))
}

/// Resolve the first present column from a synonym list, preserving real casing.
pub fn resolve_column(df: &DataFrame, candidates: &[&str]) -> Option<String> {
    let names: Vec<String> = df
        .get_column_names()
        .iter()
        .map(|n| n.as_str().to_string())
        .collect();
    candidates.iter().find_map(|c| {
        names
            .iter()
            .find(|h| h.eq_ignore_ascii_case(c))
            .cloned()
    })
}

/// Keep rows whose value in the first matching column is in `allowed`.
/// Returns None when no candidate column exists in the frame.
pub fn filter_rows_by_values(
    df: DataFrame,
    candidate_cols: &[&str],
    allowed: &[String],
) -> Result<Option<DataFrame>, String> {
    let Some(col_name) = resolve_column(&df, candidate_cols) else {
        return Ok(None);
    };

    let allowed_lower: Vec<String> = allowed
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect();
    let series = df.column(&col_name).map_err(|e| e.to_string())?;
    let mut mask = Vec::with_capacity(series.len());
    for i in 0..series.len() {
        let keep = match series.get(i).map_err(|e| e.to_string())? {
            AnyValue::Null => false,
            v => {
                let s = any_value_text(v).trim().to_lowercase();
                allowed_lower.iter().any(|a| a == &s)
            }
        };
        mask.push(keep);
    }
    let mask = BooleanChunked::new("mask".into(), &mask);
    df.filter(&mask).map(Some).map_err(|e| e.to_string())
}

/// Keep rows whose Latitude/Longitude fall inside `extent`.
/// Returns None when the frame has no lat/lon columns.
pub fn filter_rows_by_extent(
    df: DataFrame,
    extent: &ExtentMap,
) -> Result<Option<DataFrame>, String> {
    let Some(lat_name) = resolve_column(&df, LAT_COLUMNS) else {
        return Ok(None);
    };
    let Some(lon_name) = resolve_column(&df, LON_COLUMNS) else {
        return Ok(None);
    };

    let lat = df.column(&lat_name).map_err(|e| e.to_string())?;
    let lon = df.column(&lon_name).map_err(|e| e.to_string())?;
    let n = lat.len().min(lon.len());
    let mut mask = Vec::with_capacity(df.height());
    for i in 0..n {
        let keep = match (
            lat.get(i).ok().and_then(parse_coord),
            lon.get(i).ok().and_then(parse_coord),
        ) {
            (Some(la), Some(lo)) => extent.contains(la, lo),
            _ => false,
        };
        mask.push(keep);
    }
    while mask.len() < df.height() {
        mask.push(false);
    }
    let mask = BooleanChunked::new("mask".into(), &mask);
    df.filter(&mask).map(Some).map_err(|e| e.to_string())
}

/// Keep rows whose census year falls inside the inclusive bounds.
pub fn filter_rows_by_year(
    df: DataFrame,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<Option<DataFrame>, String> {
    let Some(col_name) = resolve_column(&df, YEAR_COLUMNS) else {
        return Ok(None);
    };

    let series = df.column(&col_name).map_err(|e| e.to_string())?;
    let mut mask = Vec::with_capacity(series.len());
    for i in 0..series.len() {
        let keep = match series.get(i).map_err(|e| e.to_string())? {
            AnyValue::Null => false,
            v => match parse_year(&any_value_text(v)) {
                // Unparseable years are dropped rather than silently included.
                None => false,
                Some(year) => {
                    start.map(|s| year >= s).unwrap_or(true)
                        && end.map(|e| year <= e).unwrap_or(true)
                }
            },
        };
        mask.push(keep);
    }
    let mask = BooleanChunked::new("mask".into(), &mask);
    df.filter(&mask).map(Some).map_err(|e| e.to_string())
}

/// Decimal census years (e.g. "2004.5") round down to the calendar year.
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

/// Text of a cell value without the quoting that `Display` can add for strings.
pub fn any_value_text(v: AnyValue) -> String {
    match v {
        AnyValue::String(s) => s.to_string(),
        AnyValue::StringOwned(s) => s.to_string(),
        other => other.to_string(),
    }
}
