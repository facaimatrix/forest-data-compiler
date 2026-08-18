use calamine::{open_workbook_auto, Data, Reader};
use polars::prelude::*;
use std::collections::BTreeSet;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("unsupported file extension '{0}'; expected xlsx, xls, csv, tsv, or parquet")]
    UnsupportedExtension(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Polars error: {0}")]
    Polars(#[from] PolarsError),
    #[error("XLSX error: {0}")]
    Xlsx(String),
    #[error("workbook has no sheets")]
    NoSheets,
    #[error("sheet '{0}' is empty")]
    EmptySheet(String),
}

pub fn read_file(path: &Path) -> Result<DataFrame, ReadError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let df = match ext.as_str() {
        "xlsx" | "xls" | "ods" => read_xlsx(path)?,
        "csv" => read_csv(path, b',')?,
        "tsv" => read_csv(path, b'\t')?,
        "parquet" => read_parquet(path)?,
        other => return Err(ReadError::UnsupportedExtension(other.to_string())),
    };
    Ok(normalize_dataframe(df))
}

pub fn read_headers_only(path: &Path) -> Result<Vec<String>, ReadError> {
    peek_headers(path)
}

/// Fast header peek — does not load the full table.
pub fn peek_headers(path: &Path) -> Result<Vec<String>, ReadError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "csv" => peek_csv_headers(path, b','),
        "tsv" => peek_csv_headers(path, b'\t'),
        "xlsx" | "xls" | "ods" => peek_xlsx_headers(path),
        "parquet" => {
            let df = read_parquet(path)?;
            Ok(df
                .get_column_names()
                .iter()
                .map(|s| s.to_string())
                .collect())
        }
        other => Err(ReadError::UnsupportedExtension(other.to_string())),
    }
}

/// Distinct non-empty values from the first matching column name (capped).
pub fn peek_column_uniques(
    path: &Path,
    candidate_cols: &[&str],
    max_rows: usize,
) -> Result<Vec<String>, ReadError> {
    let headers = peek_headers(path)?;
    let col = candidate_cols
        .iter()
        .find(|c| {
            headers
                .iter()
                .any(|h| h.eq_ignore_ascii_case(c))
        })
        .copied();
    let Some(col_name) = col else {
        return Ok(Vec::new());
    };

    // Resolve actual header casing
    let actual = headers
        .iter()
        .find(|h| h.eq_ignore_ascii_case(col_name))
        .cloned()
        .unwrap_or_else(|| col_name.to_string());

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let df = match ext.as_str() {
        "csv" => read_csv_n_rows(path, b',', max_rows)?,
        "tsv" => read_csv_n_rows(path, b'\t', max_rows)?,
        _ => {
            // For spreadsheet/parquet, read full then take head (usually smaller contributor files)
            let full = read_file(path)?;
            full.head(Some(max_rows))
        }
    };

    if df.width() == 0 || !df.get_column_names().iter().any(|n| n.as_str() == actual) {
        return Ok(Vec::new());
    }

    let series = df.column(&actual).map_err(ReadError::Polars)?;
    let mut set = BTreeSet::new();
    for i in 0..series.len() {
        if let Ok(v) = series.get(i) {
            let s = match v {
                AnyValue::Null => continue,
                other => crate::project_filter::any_value_text(other),
            };
            let t = s.trim();
            if !t.is_empty() && t != "null" {
                set.insert(t.to_string());
            }
        }
    }
    Ok(set.into_iter().collect())
}

fn peek_csv_headers(path: &Path, sep: u8) -> Result<Vec<String>, ReadError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    // skip UTF-8 BOM / blank lines
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(Vec::new());
        }
        let trimmed = line.trim_start_matches('\u{feff}').trim();
        if !trimmed.is_empty() {
            line = trimmed.to_string();
            break;
        }
    }
    Ok(split_csv_line(&line, sep))
}

fn split_csv_line(line: &str, sep: u8) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let sep = sep as char;
    for c in line.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if c == sep && !in_quotes {
            out.push(cur.trim().to_string());
            cur.clear();
            continue;
        }
        if c == '\r' || c == '\n' {
            continue;
        }
        cur.push(c);
    }
    out.push(cur.trim().to_string());
    out
}

fn peek_xlsx_headers(path: &Path) -> Result<Vec<String>, ReadError> {
    let mut wb = open_workbook_auto(path).map_err(|e| ReadError::Xlsx(e.to_string()))?;
    let sheet_names = wb.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Err(ReadError::NoSheets);
    }
    let preferred = sheet_names
        .iter()
        .find(|n| {
            let l = n.to_lowercase();
            l.contains("data") || l.contains("gfb") || l.contains("tree")
        })
        .cloned()
        .unwrap_or_else(|| sheet_names[0].clone());
    let range = wb
        .worksheet_range(&preferred)
        .map_err(|e| ReadError::Xlsx(e.to_string()))?;
    let mut rows = range.rows();
    let header_row = rows
        .next()
        .ok_or_else(|| ReadError::EmptySheet(preferred))?;
    Ok(header_row
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let s = cell_to_string(c);
            if s.trim().is_empty() {
                format!("_unnamed_{}", i + 1)
            } else {
                s.trim().to_string()
            }
        })
        .collect())
}

fn read_csv_n_rows(path: &Path, sep: u8, n: usize) -> Result<DataFrame, ReadError> {
    let df = CsvReadOptions::default()
        .with_has_header(true)
        .with_infer_schema_length(Some(200))
        .with_n_rows(Some(n))
        .map_parse_options(|opts| opts.with_separator(sep))
        .try_into_reader_with_file_path(Some(path.to_path_buf()))?
        .finish()?;
    Ok(df)
}

fn read_csv(path: &Path, sep: u8) -> Result<DataFrame, ReadError> {
    let df = CsvReadOptions::default()
        .with_has_header(true)
        .with_infer_schema_length(Some(5000))
        .map_parse_options(|opts| opts.with_separator(sep))
        .try_into_reader_with_file_path(Some(path.to_path_buf()))?
        .finish()?;
    Ok(df)
}

fn read_parquet(path: &Path) -> Result<DataFrame, ReadError> {
    let df = ParquetReader::new(std::fs::File::open(path)?).finish()?;
    Ok(df)
}

fn read_xlsx(path: &Path) -> Result<DataFrame, ReadError> {
    let mut wb = open_workbook_auto(path).map_err(|e| ReadError::Xlsx(e.to_string()))?;
    let sheet_names = wb.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Err(ReadError::NoSheets);
    }

    // Prefer a sheet that looks like tree data (DataTemplate / first non-empty).
    let preferred = sheet_names
        .iter()
        .find(|n| {
            let l = n.to_lowercase();
            l.contains("data") || l.contains("gfb") || l.contains("tree")
        })
        .cloned()
        .unwrap_or_else(|| sheet_names[0].clone());

    let range = wb
        .worksheet_range(&preferred)
        .map_err(|e| ReadError::Xlsx(e.to_string()))?;

    let mut rows = range.rows();
    let header_row = rows.next().ok_or_else(|| ReadError::EmptySheet(preferred.clone()))?;
    let headers: Vec<String> = header_row
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let s = cell_to_string(c);
            if s.trim().is_empty() {
                format!("_unnamed_{}", i + 1)
            } else {
                s.trim().to_string()
            }
        })
        .collect();

    let mut columns: Vec<Vec<Option<String>>> = headers.iter().map(|_| Vec::new()).collect();
    for row in rows {
        // Skip fully empty rows
        if row.iter().all(|c| matches!(c, Data::Empty)) {
            continue;
        }
        for (i, cell) in row.iter().enumerate() {
            if i >= columns.len() {
                break;
            }
            columns[i].push(match cell {
                Data::Empty => None,
                other => {
                    let s = cell_to_string(other);
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                }
            });
        }
        // pad short rows
        for col in columns.iter_mut().take(headers.len()).skip(row.len()) {
            col.push(None);
        }
    }

    let series: Vec<Column> = headers
        .into_iter()
        .zip(columns)
        .map(|(name, vals)| Series::new(name.into(), vals).into())
        .collect();

    DataFrame::new(series).map_err(ReadError::Polars)
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERR:{e}"),
    }
}

fn normalize_dataframe(df: DataFrame) -> DataFrame {
    let height = df.height();
    if height == 0 {
        return df;
    }

    let non_empty: Vec<Column> = df
        .get_columns()
        .iter()
        .filter(|s| s.null_count() < height)
        .cloned()
        .map(Column::from)
        .collect();

    if non_empty.is_empty() {
        return df;
    }

    let mut df = DataFrame::new(non_empty).unwrap_or(df);
    let old_names: Vec<String> = df
        .get_column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let new_names: Vec<String> = old_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let trimmed = name.trim();
            let base = if trimmed.is_empty() || name.starts_with("column_") {
                format!("_unnamed_{}", i + 1)
            } else {
                trimmed.to_string()
            };
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{base}_{count}")
            }
        })
        .collect();

    let _ = df.set_column_names(&new_names);
    df
}

pub fn is_supported_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "csv" | "tsv" | "xlsx" | "xls" | "ods" | "parquet"
    )
}
