use crate::geo::{GeoFilter, GeoMode, BIOREGION_COLUMNS, COUNTRY_COLUMNS};
use crate::manifest::CompileManifest;
use crate::project_filter::{
    any_value_text, apply_manifest_filters, filter_rows_by_extent, filter_rows_by_values,
};
use crate::raster_lookup::ExtentMap;
use crate::reader::read_file;
use chrono::Utc;
use polars::prelude::*;
use rust_xlsxwriter::Workbook;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompileFormat {
    Csv,
    Xlsx,
    Parquet,
    ZipOriginals,
    Auto,
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub format: CompileFormat,
    pub add_source_column: bool,
    pub geo: GeoFilter,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            format: CompileFormat::Auto,
            add_source_column: true,
            geo: GeoFilter::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CompileReport {
    pub project_id: String,
    pub project_title: String,
    pub compiled_at: String,
    pub output_path: String,
    pub format: String,
    pub source_count: usize,
    pub total_rows: usize,
    pub sources: Vec<SourceStat>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStat {
    pub path: String,
    pub file_name: String,
    pub rows: usize,
    pub columns: usize,
}

pub fn compile_files(
    paths: &[PathBuf],
    output: &Path,
    manifest: &CompileManifest,
    opts: &CompileOptions,
) -> Result<CompileReport, String> {
    if paths.is_empty() {
        return Err("No files selected to compile".into());
    }

    let format = resolve_format(opts.format, paths);
    let mut notes = Vec::new();
    notes.push(format!(
        "Compiled for Forest Data Exchange project '{}' ({})",
        manifest.project.title, manifest.project.id
    ));
    match opts.geo.mode {
        GeoMode::Global => notes.push("Geography filter: Global (no row filter)".into()),
        GeoMode::Bioregions => notes.push(format!(
            "Geography filter: Bioregions ({})",
            if opts.geo.bioregions.is_empty() {
                "any".into()
            } else {
                opts.geo.bioregions.join(", ")
            }
        )),
        GeoMode::ByCountry => notes.push(format!(
            "Geography filter: By country ({})",
            if opts.geo.countries.is_empty() {
                "any".into()
            } else {
                opts.geo.countries.join(", ")
            }
        )),
        GeoMode::Shapefile => notes.push(format!(
            "Geography filter: plots inside {}",
            opts.geo
                .shapefile_path
                .as_deref()
                .map(|p| Path::new(p)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(p)
                    .to_string())
                .unwrap_or_else(|| "shapefile".into())
        )),
    }

    let (total_rows, sources) = match format {
        CompileFormat::ZipOriginals => {
            write_zip_originals(paths, output)?;
            notes.push("Output is a ZIP of original files (mixed formats).".into());
            let sources: Vec<SourceStat> = paths
                .iter()
                .map(|p| SourceStat {
                    path: p.display().to_string(),
                    file_name: p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string(),
                    rows: 0,
                    columns: 0,
                })
                .collect();
            (0, sources)
        }
        other => {
            let (df, sources, filter_notes) =
                concat_tables(paths, opts.add_source_column, &opts.geo, manifest)?;
            notes.extend(filter_notes);
            let rows = df.height();
            match other {
                CompileFormat::Csv => write_csv(&df, output)?,
                CompileFormat::Xlsx => write_xlsx(&df, output)?,
                CompileFormat::Parquet => write_parquet(&df, output)?,
                CompileFormat::Auto | CompileFormat::ZipOriginals => unreachable!(),
            }
            notes.push(format!("Merged {rows} rows from {} source file(s).", sources.len()));
            (rows, sources)
        }
    };

    // Sidecar report next to output
    let report = CompileReport {
        project_id: manifest.project.id.clone(),
        project_title: manifest.project.title.clone(),
        compiled_at: Utc::now().to_rfc3339(),
        output_path: output.display().to_string(),
        format: format_label(format).into(),
        source_count: sources.len(),
        total_rows,
        sources,
        notes: notes.clone(),
    };

    let sidecar = report_sidecar_path(output);
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(&sidecar, json).map_err(|e| e.to_string())?;

    Ok(report)
}

fn resolve_format(requested: CompileFormat, paths: &[PathBuf]) -> CompileFormat {
    match requested {
        CompileFormat::Auto => {
            let all_csv = paths.iter().all(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str(),
                    "csv" | "tsv"
                )
            });
            if all_csv {
                CompileFormat::Csv
            } else {
                // Prefer merged xlsx when we can read everything
                CompileFormat::Xlsx
            }
        }
        other => other,
    }
}

fn format_label(f: CompileFormat) -> &'static str {
    match f {
        CompileFormat::Csv => "csv",
        CompileFormat::Xlsx => "xlsx",
        CompileFormat::Parquet => "parquet",
        CompileFormat::ZipOriginals => "zip",
        CompileFormat::Auto => "auto",
    }
}

fn report_sidecar_path(output: &Path) -> PathBuf {
    let mut s = output.as_os_str().to_owned();
    s.push(".compile-report.json");
    PathBuf::from(s)
}

fn concat_tables(
    paths: &[PathBuf],
    add_source: bool,
    geo: &GeoFilter,
    manifest: &CompileManifest,
) -> Result<(DataFrame, Vec<SourceStat>, Vec<String>), String> {
    let mut frames = Vec::new();
    let mut sources = Vec::new();
    let mut all_cols: BTreeSet<String> = BTreeSet::new();
    let mut filter_notes: Vec<String> = Vec::new();

    let extent = if geo.mode == GeoMode::Shapefile {
        let path = geo
            .shapefile_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "Choose a reference shapefile to select plots by Latitude/Longitude".to_string()
            })?;
        Some(ExtentMap::from_path(Path::new(path))?)
    } else {
        None
    };

    for path in paths {
        let label = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut df = read_file(path).map_err(|e| format!("{}: {e}", path.display()))?;

        let (filtered, notes) = apply_manifest_filters(df, manifest)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        df = filtered;
        for note in notes {
            filter_notes.push(format!("{label}: {note}"));
        }

        df = apply_geo_row_filter(df, geo, extent.as_ref())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if df.height() == 0 {
            filter_notes.push(format!("{label}: no rows matched the project filters"));
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        if add_source {
            df = df
                .lazy()
                .with_column(lit(file_name.clone()).alias("compile_source"))
                .collect()
                .map_err(|e| e.to_string())?;
        }

        for name in df.get_column_names() {
            all_cols.insert(name.to_string());
        }

        sources.push(SourceStat {
            path: path.display().to_string(),
            file_name,
            rows: df.height(),
            columns: df.width(),
        });
        frames.push(df);
    }

    if frames.is_empty() {
        let detail = if filter_notes.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", filter_notes.join("\n"))
        };
        return Err(format!(
            "No rows left after the project filters (geography / forest type / year range). \
             Widen the filter or select different files.{detail}"
        ));
    }

    let col_order: Vec<String> = {
        let mut cols: Vec<String> = all_cols.into_iter().collect();
        if let Some(i) = cols.iter().position(|c| c == "compile_source") {
            let c = cols.remove(i);
            cols.push(c);
        }
        cols
    };

    let aligned: Vec<DataFrame> = frames
        .into_iter()
        .map(|df| align_to_string_schema(df, &col_order))
        .collect::<Result<_, _>>()?;

    let mut iter = aligned.into_iter();
    let first = iter.next().ok_or_else(|| "No frames".to_string())?;
    let mut combined = first;
    for df in iter {
        combined = combined.vstack(&df).map_err(|e| e.to_string())?;
    }

    Ok((combined, sources, filter_notes))
}

fn apply_geo_row_filter(
    df: DataFrame,
    geo: &GeoFilter,
    extent: Option<&ExtentMap>,
) -> Result<DataFrame, String> {
    match geo.mode {
        GeoMode::Global => Ok(df),
        GeoMode::Shapefile => {
            let Some(extent) = extent else {
                return Err(
                    "Choose a reference shapefile to select plots by Latitude/Longitude".into(),
                );
            };
            match filter_rows_by_extent(df.clone(), extent)? {
                Some(filtered) => Ok(filtered),
                // No Latitude/Longitude columns: fail closed rather than keep every row.
                None => Ok(df.head(Some(0))),
            }
        }
        GeoMode::Bioregions if !geo.bioregions.is_empty() => {
            Ok(filter_rows_by_values(df.clone(), BIOREGION_COLUMNS, &geo.bioregions)?.unwrap_or(df))
        }
        GeoMode::ByCountry if !geo.countries.is_empty() => {
            Ok(filter_rows_by_values(df.clone(), COUNTRY_COLUMNS, &geo.countries)?.unwrap_or(df))
        }
        _ => Ok(df),
    }
}

fn align_to_string_schema(df: DataFrame, cols: &[String]) -> Result<DataFrame, String> {
    let existing: std::collections::HashSet<String> = df
        .get_column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut df = df;
    // Cast existing to string
    let cast_exprs: Vec<Expr> = df
        .get_column_names()
        .iter()
        .map(|n| {
            let name = n.as_str();
            col(name).cast(DataType::String).alias(name)
        })
        .collect();
    df = df
        .lazy()
        .with_columns(cast_exprs)
        .collect()
        .map_err(|e| e.to_string())?;

    for c in cols {
        if !existing.contains(c) {
            df = df
                .lazy()
                .with_column(lit(NULL).cast(DataType::String).alias(c.as_str()))
                .collect()
                .map_err(|e| e.to_string())?;
        }
    }

    df = df.select(cols).map_err(|e| e.to_string())?;
    Ok(df)
}

fn write_csv(df: &DataFrame, path: &Path) -> Result<(), String> {
    let mut df = df.clone();
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    CsvWriter::new(&mut file)
        .include_header(true)
        .finish(&mut df)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn write_parquet(df: &DataFrame, path: &Path) -> Result<(), String> {
    let mut df = df.clone();
    let file = File::create(path).map_err(|e| e.to_string())?;
    ParquetWriter::new(file)
        .finish(&mut df)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn write_xlsx(df: &DataFrame, path: &Path) -> Result<(), String> {
    let mut wb = Workbook::new();
    let sheet = wb
        .add_worksheet()
        .set_name("Compiled")
        .map_err(|e| e.to_string())?;

    let names: Vec<String> = df
        .get_column_names()
        .iter()
        .map(|n| n.as_str().to_string())
        .collect();
    for (c, name) in names.iter().enumerate() {
        sheet
            .write_string(0, c as u16, name.as_str())
            .map_err(|e| e.to_string())?;
    }

    let height = df.height();
    for r in 0..height {
        for (c, name) in names.iter().enumerate() {
            let col = df.column(name.as_str()).map_err(|e| e.to_string())?;
            let val = col.get(r).map_err(|e| e.to_string())?;
            let text = match val {
                AnyValue::Null => continue,
                other => any_value_text(other),
            };
            sheet
                .write_string((r + 1) as u32, c as u16, &text)
                .map_err(|e| e.to_string())?;
        }
    }

    wb.save(path).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_zip_originals(paths: &[PathBuf], output: &Path) -> Result<(), String> {
    let file = File::create(output).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in paths {
        let base = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.bin")
            .to_string();
        let mut name = base.clone();
        let mut i = 2;
        while used_names.contains(&name) {
            name = format!("{i}_{base}");
            i += 1;
        }
        used_names.insert(name.clone());

        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        zip.start_file(&name, opts).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn default_output_name(manifest: &CompileManifest, format: CompileFormat) -> String {
    let slug = slugify(&manifest.project.title);
    let id_short = manifest.project.id.chars().take(8).collect::<String>();
    let stamp = Utc::now().format("%Y%m%d");
    let ext = match format {
        CompileFormat::Csv | CompileFormat::Auto => "csv",
        CompileFormat::Xlsx => "xlsx",
        CompileFormat::Parquet => "parquet",
        CompileFormat::ZipOriginals => "zip",
    };
    format!("forest-data-exchange-compiled-{slug}-{id_short}-{stamp}.{ext}")
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "project".into()
    } else {
        trimmed.chars().take(48).collect()
    }
}
