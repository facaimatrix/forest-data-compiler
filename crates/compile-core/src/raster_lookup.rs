//! Sample classified GeoTIFFs or shapefiles at plot coordinates.
//!
//! Each layer (ecoregion, forest type) can be a north-up WGS84 GeoTIFF or a
//! polygon shapefile. FAO Global Ecological Zones work well as a shapefile;
//! a GEZ GeoTIFF still works. Optional `{stem}.legend.json` maps pixel codes
//! or attribute values onto Forest Data Exchange bioregions / forest types.
//!
//! Layers are not shipped with the app — pick files in Dataset metadata, drop
//! them in `{data}/rasters`, or set `FOREST_DATA_COMPILER_RASTERS`.

use crate::geo::{BIOREGIONS, FOREST_TYPES};
use geo::algorithm::bounding_rect::BoundingRect;
use geo::algorithm::contains::Contains;
use geo::{MultiPolygon, Point as GeoPoint};
use georaster::geotiff::{GeoTiffReader, RasterValue};
use georaster::Coordinate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_SAMPLE_POINTS: usize = 4_000;

const ECOREGION_FIELDS: &[&str] = &[
    "gez_name", "GEZ_NAME", "gez_code", "GEZ", "gez", "ECO_NAME", "BIOME_NAME",
    "Bioregion", "Ecoregion", "ECOREGION", "biome", "NAME", "Class", "CLASS",
    "label", "GRIDCODE", "Value", "VALUE",
];

const FOREST_TYPE_FIELDS: &[&str] = &[
    "ForestType", "FORESTTYPE", "forest_type", "FORTYPE", "TYPE", "Class",
    "CLASS", "NAME", "gez_name", "GEZ", "label", "GRIDCODE", "Value", "VALUE",
];

#[derive(Debug, Clone, Default)]
pub struct LayerSources {
    pub folder: Option<PathBuf>,
    pub ecoregion: Option<PathBuf>,
    pub forest_type: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RasterLegend {
    #[serde(default)]
    pub nodata: Option<i64>,
    #[serde(default)]
    pub classes: HashMap<String, LegendClass>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LegendClass {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub bioregion: Option<String>,
    #[serde(default)]
    pub forest_types: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RasterSuggestions {
    pub ecoregions: Vec<String>,
    pub forest_types: Vec<String>,
    pub gez_labels: Vec<String>,
    pub ecoregion_source: Option<String>,
    pub forest_type_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RasterCatalogStatus {
    pub folder: Option<String>,
    pub ecoregion_path: Option<String>,
    pub forest_type_path: Option<String>,
    pub ecoregion_kind: Option<String>,
    pub forest_type_kind: Option<String>,
    pub using_fao_gez_legend: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RasterCatalog {
    folder: Option<PathBuf>,
    ecoregion: Option<ClassifiedLayer>,
    forest_type: Option<ClassifiedLayer>,
    using_fao_gez_legend: bool,
}

#[derive(Debug, Clone)]
struct ClassifiedLayer {
    path: PathBuf,
    legend: RasterLegend,
    shapefile: Option<Vec<ShapeFeature>>,
}

#[derive(Debug, Clone)]
struct ShapeFeature {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    geom: MultiPolygon<f64>,
    class: LegendClass,
}

impl RasterLegend {
    pub fn lookup(&self, code: i64) -> Option<&LegendClass> {
        if self.nodata == Some(code) {
            return None;
        }
        self.classes
            .get(&code.to_string())
            .or_else(|| self.classes.get(&format!("{code}.0")))
    }

    pub fn lookup_token(&self, token: &str) -> Option<&LegendClass> {
        let t = token.trim();
        if t.is_empty() {
            return None;
        }
        if let Ok(n) = t.parse::<i64>() {
            if let Some(c) = self.lookup(n) {
                return Some(c);
            }
        }
        self.classes
            .get(t)
            .or_else(|| self.classes.get(&t.to_ascii_lowercase()))
            .or_else(|| {
                self.classes
                    .values()
                    .find(|c| c.label.eq_ignore_ascii_case(t))
            })
    }

    fn from_path(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }
}

impl RasterCatalog {
    pub fn load(sources: &LayerSources, data_folder: Option<&Path>) -> Option<Self> {
        let mut using_fao = false;
        let mut ecoregion = sources
            .ecoregion
            .as_ref()
            .and_then(|p| ClassifiedLayer::open(p, LayerKind::Ecoregion, &mut using_fao));
        let mut forest_type = sources
            .forest_type
            .as_ref()
            .and_then(|p| ClassifiedLayer::open(p, LayerKind::ForestType, &mut using_fao));

        let mut folder = sources.folder.clone();
        if ecoregion.is_none() || forest_type.is_none() {
            let dirs = raster_search_dirs(sources.folder.as_deref(), data_folder);
            if let Some(discovered) = Self::discover(&dirs) {
                if folder.is_none() {
                    folder = discovered.folder;
                }
                using_fao |= discovered.using_fao_gez_legend;
                if ecoregion.is_none() {
                    ecoregion = discovered.ecoregion;
                }
                if forest_type.is_none() {
                    forest_type = discovered.forest_type;
                }
            }
        }
        if ecoregion.is_none() && forest_type.is_none() {
            return None;
        }
        Some(Self {
            folder,
            ecoregion,
            forest_type,
            using_fao_gez_legend: using_fao,
        })
    }

    pub fn discover(dirs: &[PathBuf]) -> Option<Self> {
        for dir in dirs {
            if let Some(catalog) = Self::from_dir(dir) {
                return Some(catalog);
            }
        }
        None
    }

    pub fn from_dir(dir: &Path) -> Option<Self> {
        if !dir.is_dir() {
            return None;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| is_layer_file(p))
            .collect();
        if files.is_empty() {
            return None;
        }
        files.sort();

        let mut ecoregion = None;
        let mut forest_type = None;
        let mut using_fao = false;

        for path in &files {
            match classify_filename(path) {
                LayerKind::ForestType if forest_type.is_none() => {
                    forest_type = ClassifiedLayer::open(path, LayerKind::ForestType, &mut using_fao);
                }
                LayerKind::Ecoregion if ecoregion.is_none() => {
                    ecoregion = ClassifiedLayer::open(path, LayerKind::Ecoregion, &mut using_fao);
                }
                _ => {}
            }
        }
        if ecoregion.is_none() && forest_type.is_none() {
            ecoregion = ClassifiedLayer::open(&files[0], LayerKind::Ecoregion, &mut using_fao);
        }

        if ecoregion.is_none() && forest_type.is_none() {
            return None;
        }
        Some(Self {
            folder: Some(dir.to_path_buf()),
            ecoregion,
            forest_type,
            using_fao_gez_legend: using_fao,
        })
    }

    pub fn status(&self) -> RasterCatalogStatus {
        let ecoregion_path = self.ecoregion.as_ref().map(|l| l.path.display().to_string());
        let forest_type_path = self
            .forest_type
            .as_ref()
            .map(|l| l.path.display().to_string());
        let ecoregion_kind = self.ecoregion.as_ref().map(|l| l.format_name().to_string());
        let forest_type_kind = self.forest_type.as_ref().map(|l| l.format_name().to_string());
        let message = match (&ecoregion_path, &forest_type_path, &ecoregion_kind) {
            (Some(_), Some(_), _) => {
                "Ecoregion layer fills bioregion + climate forest types; forest-type layer adds extra classes".into()
            }
            (Some(_), None, Some(kind)) if kind == "shapefile" => {
                "FAO/ecoregion shapefile fills both bioregion and climate forest types (Tropical, Boreal, …)".into()
            }
            (Some(_), None, _) => {
                "Ecoregion raster fills both bioregion and climate forest types".into()
            }
            (None, Some(_), _) => "Forest-type layer only — ecoregion/bioregion will stay empty unless you pick an ecoregion file".into(),
            (None, None, _) => "No shapefile or GeoTIFF selected".into(),
        };
        RasterCatalogStatus {
            folder: self.folder.as_ref().map(|p| p.display().to_string()),
            ecoregion_path,
            forest_type_path,
            ecoregion_kind,
            forest_type_kind,
            using_fao_gez_legend: self.using_fao_gez_legend,
            message,
        }
    }

    pub fn suggest(&self, points: &[(f64, f64)]) -> RasterSuggestions {
        let sample = downsample_points(points);
        let mut ecoregions = BTreeSet::new();
        let mut forest_types = BTreeSet::new();
        let mut gez_labels = BTreeSet::new();

        if let Some(layer) = &self.ecoregion {
            for class in layer.sample_classes(&sample) {
                if !class.label.is_empty() {
                    gez_labels.insert(class.label);
                }
                if let Some(bio) = class.bioregion.filter(|s| !s.is_empty()) {
                    ecoregions.insert(bio);
                }
                for ft in class.forest_types {
                    if !ft.is_empty() {
                        forest_types.insert(ft);
                    }
                }
            }
        }
        if let Some(layer) = &self.forest_type {
            for class in layer.sample_classes(&sample) {
                if !class.label.is_empty() && class.forest_types.is_empty() {
                    forest_types.insert(class.label);
                }
                for ft in class.forest_types {
                    if !ft.is_empty() {
                        forest_types.insert(ft);
                    }
                }
            }
        }

        let ecoregion_source = self.ecoregion.as_ref().map(|l| l.format_name().to_string());
        let forest_type_source = match (self.ecoregion.is_some(), self.forest_type.is_some()) {
            (true, true) => Some("combined".into()),
            (false, true) => self.forest_type.as_ref().map(|l| l.format_name().to_string()),
            (true, false) => ecoregion_source.clone(),
            (false, false) => None,
        };

        RasterSuggestions {
            ecoregions: ecoregions.into_iter().collect(),
            forest_types: forest_types.into_iter().collect(),
            gez_labels: gez_labels.into_iter().collect(),
            ecoregion_source,
            forest_type_source,
        }
    }
}

impl ClassifiedLayer {
    fn open(path: &Path, kind: LayerKind, using_fao: &mut bool) -> Option<Self> {
        let path = normalize_layer_path(path)?;
        let legend = load_legend(&path, kind, using_fao);
        if is_shapefile(&path) {
            let shapefile = load_shapefile_features(&path, &legend, kind);
            if shapefile.is_empty() {
                return None;
            }
            Some(Self {
                path,
                legend,
                shapefile: Some(shapefile),
            })
        } else if is_geotiff(&path) {
            Some(Self {
                path,
                legend,
                shapefile: None,
            })
        } else {
            None
        }
    }

    fn format_name(&self) -> &'static str {
        if self.shapefile.is_some() {
            "shapefile"
        } else {
            "geotiff"
        }
    }

    fn sample_classes(&self, points: &[(f64, f64)]) -> Vec<LegendClass> {
        if let Some(features) = &self.shapefile {
            return sample_shapefile(features, points);
        }
        sample_geotiff(&self.path, &self.legend, points)
    }
}

fn sample_geotiff(path: &Path, legend: &RasterLegend, points: &[(f64, f64)]) -> Vec<LegendClass> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut tiff = match GeoTiffReader::open(BufReader::new(file)) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut counts: BTreeMap<i64, u32> = BTreeMap::new();
    for (lat, lon) in points {
        if !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        let value = tiff.read_pixel_at_location(Coordinate { x: *lon, y: *lat });
        if let Some(code) = raster_code(&value) {
            *counts.entry(code).or_insert(0) += 1;
        }
    }
    counts
        .into_keys()
        .filter_map(|code| legend.lookup(code).cloned())
        .collect()
}

fn sample_shapefile(features: &[ShapeFeature], points: &[(f64, f64)]) -> Vec<LegendClass> {
    let mut seen: BTreeMap<String, LegendClass> = BTreeMap::new();
    for (lat, lon) in points {
        if !lon.is_finite() || !lat.is_finite() {
            continue;
        }
        let pt = GeoPoint::new(*lon, *lat);
        for feat in features {
            if *lon < feat.min_x || *lon > feat.max_x || *lat < feat.min_y || *lat > feat.max_y {
                continue;
            }
            if feat.geom.contains(&pt) {
                let key = feat
                    .class
                    .label
                    .clone()
                    .if_empty(|| feat.class.bioregion.clone().unwrap_or_default());
                if !key.is_empty() {
                    seen.entry(key).or_insert_with(|| feat.class.clone());
                }
                break;
            }
        }
    }
    seen.into_values().collect()
}

trait IfEmpty {
    fn if_empty(self, f: impl FnOnce() -> String) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, f: impl FnOnce() -> String) -> String {
        if self.is_empty() {
            f()
        } else {
            self
        }
    }
}

fn load_shapefile_features(path: &Path, legend: &RasterLegend, kind: LayerKind) -> Vec<ShapeFeature> {
    let mut reader = match shapefile::Reader::from_path(path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let fields = match kind {
        LayerKind::Ecoregion => ECOREGION_FIELDS,
        LayerKind::ForestType => FOREST_TYPE_FIELDS,
    };
    let mut out = Vec::new();
    for item in reader.iter_shapes_and_records() {
        let Ok((shape, record)) = item else {
            continue;
        };
        let Some(geom) = shape_to_multipolygon(shape) else {
            continue;
        };
        let Some(token) = record_token(&record, fields) else {
            continue;
        };
        let Some(class) = resolve_class(&token, legend, kind) else {
            continue;
        };
        let (min_x, min_y, max_x, max_y) = match geom.bounding_rect() {
            Some(r) => (r.min().x, r.min().y, r.max().x, r.max().y),
            None => continue,
        };
        out.push(ShapeFeature {
            min_x,
            min_y,
            max_x,
            max_y,
            geom,
            class,
        });
    }
    out
}

fn shape_to_multipolygon(shape: shapefile::Shape) -> Option<MultiPolygon<f64>> {
    match shape {
        shapefile::Shape::Polygon(p) => MultiPolygon::<f64>::try_from(p).ok(),
        shapefile::Shape::PolygonZ(p) => MultiPolygon::<f64>::try_from(p).ok(),
        shapefile::Shape::PolygonM(p) => MultiPolygon::<f64>::try_from(p).ok(),
        _ => None,
    }
}

fn record_token(record: &shapefile::dbase::Record, preferred: &[&str]) -> Option<String> {
    for name in preferred {
        if let Some(value) = record.get(*name).or_else(|| {
            record.as_ref().iter().find_map(|(k, v)| {
                if k.trim().eq_ignore_ascii_case(name) {
                    Some(v)
                } else {
                    None
                }
            })
        }) {
            if let Some(s) = field_to_string(value) {
                return Some(s);
            }
        }
    }
    record.as_ref().iter().find_map(|(_, v)| field_to_string(v))
}

fn field_to_string(value: &shapefile::dbase::FieldValue) -> Option<String> {
    let s = match value {
        shapefile::dbase::FieldValue::Character(v) => v.as_ref()?.trim().to_string(),
        shapefile::dbase::FieldValue::Numeric(v) => format_num(*v.as_ref()? as f64),
        shapefile::dbase::FieldValue::Float(v) => format_num(f64::from(*v.as_ref()?)),
        shapefile::dbase::FieldValue::Integer(n) => n.to_string(),
        shapefile::dbase::FieldValue::Double(n) => format_num(*n),
        shapefile::dbase::FieldValue::Memo(v) => v.trim().to_string(),
        _ => return None,
    };
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

fn resolve_class(raw: &str, legend: &RasterLegend, kind: LayerKind) -> Option<LegendClass> {
    if let Some(c) = legend.lookup_token(raw) {
        return Some(c.clone());
    }
    if kind == LayerKind::Ecoregion {
        if let Some(c) = fao_legend().lookup_token(raw) {
            return Some(c.clone());
        }
    }
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(bio) = BIOREGIONS
        .iter()
        .find(|b| b.eq_ignore_ascii_case(t))
        .map(|b| (*b).to_string())
    {
        return Some(LegendClass {
            label: bio.clone(),
            bioregion: Some(bio),
            forest_types: Vec::new(),
        });
    }
    if let Some(ft) = FOREST_TYPES
        .iter()
        .find(|b| b.eq_ignore_ascii_case(t))
        .map(|b| (*b).to_string())
    {
        return Some(LegendClass {
            label: ft.clone(),
            bioregion: None,
            forest_types: vec![ft],
        });
    }
    Some(LegendClass {
        label: t.to_string(),
        bioregion: if kind == LayerKind::Ecoregion {
            Some(t.to_string())
        } else {
            None
        },
        forest_types: if kind == LayerKind::ForestType {
            vec![t.to_string()]
        } else {
            Vec::new()
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerKind {
    Ecoregion,
    ForestType,
}

fn classify_filename(path: &Path) -> LayerKind {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.contains("forest") && (name.contains("type") || name.contains("fty")) {
        LayerKind::ForestType
    } else {
        LayerKind::Ecoregion
    }
}

fn is_layer_file(path: &Path) -> bool {
    is_geotiff(path) || is_shapefile(path)
}

fn is_geotiff(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()),
        Some(ext) if ext == "tif" || ext == "tiff"
    )
}

fn is_shapefile(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("shp"))
        .unwrap_or(false)
}

fn normalize_layer_path(path: &Path) -> Option<PathBuf> {
    if path.is_file() && (is_geotiff(path) || is_shapefile(path)) {
        return Some(path.to_path_buf());
    }
    if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("dbf") || e.eq_ignore_ascii_case("shx") || e.eq_ignore_ascii_case("prj")).unwrap_or(false) {
        let shp = path.with_extension("shp");
        if shp.is_file() {
            return Some(shp);
        }
    }
    None
}

fn load_legend(layer: &Path, kind: LayerKind, using_fao: &mut bool) -> RasterLegend {
    let stem = layer.with_extension("");
    let candidates = [
        layer.with_extension("legend.json"),
        PathBuf::from(format!("{}.legend.json", stem.display())),
        layer.with_file_name("legend.json"),
    ];
    for path in candidates {
        if let Some(legend) = RasterLegend::from_path(&path) {
            if !legend.classes.is_empty() {
                return legend;
            }
        }
    }
    let name = layer
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let looks_fao = name.contains("gez")
        || name.contains("ecozone")
        || name.contains("fao")
        || name.contains("ecoregion")
        || name.contains("bioregion");
    if kind == LayerKind::Ecoregion && (looks_fao || is_shapefile(layer)) {
        *using_fao = true;
        return fao_gez_legend();
    }
    if kind == LayerKind::Ecoregion {
        *using_fao = true;
        fao_gez_legend()
    } else {
        RasterLegend::default()
    }
}

fn raster_code(value: &RasterValue) -> Option<i64> {
    Some(match value {
        RasterValue::NoData => return None,
        RasterValue::U8(v) => i64::from(*v),
        RasterValue::U16(v) => i64::from(*v),
        RasterValue::U32(v) => i64::from(*v),
        RasterValue::U64(v) => i64::try_from(*v).ok()?,
        RasterValue::F32(v) if v.is_finite() => v.round() as i64,
        RasterValue::F64(v) if v.is_finite() => v.round() as i64,
        RasterValue::I8(v) => i64::from(*v),
        RasterValue::I16(v) => i64::from(*v),
        RasterValue::I32(v) => i64::from(*v),
        RasterValue::I64(v) => *v,
        _ => return None,
    })
}

fn downsample_points(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (lat, lon) in points {
        let key = ((lat * 20.0).round() as i64, (lon * 20.0).round() as i64);
        if seen.insert(key) {
            out.push((*lat, *lon));
        }
        if out.len() >= MAX_SAMPLE_POINTS {
            break;
        }
    }
    out
}

fn fao_legend() -> &'static RasterLegend {
    static LEGEND: OnceLock<RasterLegend> = OnceLock::new();
    LEGEND.get_or_init(fao_gez_legend)
}

/// Sequential 1–21 codes used by common FAO GEZ 2010 rasters (Earth Map / FRA order).
/// Shapefile attributes `gez_name` / `gez_code` (e.g. TAr) use the same classes.
pub fn fao_gez_legend() -> RasterLegend {
    let class = |label: &str, bioregion: Option<&str>, forest_types: &[&str]| LegendClass {
        label: label.into(),
        bioregion: bioregion.map(|s| s.to_string()),
        forest_types: forest_types.iter().map(|s| (*s).to_string()).collect(),
    };
    let mut classes = HashMap::new();
    let rows: &[(i64, &str, LegendClass)] = &[
        (1, "TAr", class("Tropical rain forest", Some("Tropical moist broadleaf forests"), &["Tropical"])),
        (2, "TAwa", class("Tropical moist deciduous forest", Some("Tropical moist broadleaf forests"), &["Tropical"])),
        (3, "TAwb", class("Tropical dry forest", Some("Tropical dry broadleaf forests"), &["Tropical", "Dry forest"])),
        (4, "TBSh", class("Tropical shrubland", Some("Tropical dry broadleaf forests"), &["Tropical", "Dry forest"])),
        (5, "TBWh", class("Tropical desert", None, &["Tropical"])),
        (6, "TM", class("Tropical mountain systems", Some("Tropical moist broadleaf forests"), &["Tropical", "Montane"])),
        (7, "SCf", class("Subtropical humid forest", Some("Temperate broadleaf & mixed forests"), &["Subtropical"])),
        (8, "SCs", class("Subtropical dry forest", Some("Mediterranean forests"), &["Subtropical", "Mediterranean", "Dry forest"])),
        (9, "SBSh", class("Subtropical steppe", Some("Mediterranean forests"), &["Subtropical", "Dry forest"])),
        (10, "SBWh", class("Subtropical desert", None, &["Subtropical"])),
        (11, "SM", class("Subtropical mountain systems", Some("Temperate coniferous forests"), &["Subtropical", "Montane"])),
        (12, "TeDo", class("Temperate oceanic forest", Some("Temperate broadleaf & mixed forests"), &["Temperate"])),
        (13, "TeDc", class("Temperate continental forest", Some("Temperate broadleaf & mixed forests"), &["Temperate"])),
        (14, "TeBSk", class("Temperate steppe", Some("Temperate broadleaf & mixed forests"), &["Temperate"])),
        (15, "TeBWk", class("Temperate desert", None, &["Temperate"])),
        (16, "TeM", class("Temperate mountain systems", Some("Temperate coniferous forests"), &["Temperate", "Montane"])),
        (17, "Ba", class("Boreal coniferous forest", Some("Boreal forests/taiga"), &["Boreal", "Coniferous"])),
        (18, "Bb", class("Boreal tundra woodland", Some("Boreal forests/taiga"), &["Boreal"])),
        (19, "BM", class("Boreal mountain systems", Some("Boreal forests/taiga"), &["Boreal", "Montane"])),
        (20, "P", class("Polar", None, &[])),
        (21, "Water", class("Water", None, &[])),
    ];
    for (code, gez, row) in rows {
        classes.insert(code.to_string(), row.clone());
        classes.insert((*gez).to_string(), row.clone());
        classes.insert(gez.to_ascii_lowercase(), row.clone());
        classes.insert(row.label.to_ascii_lowercase(), row.clone());
    }
    RasterLegend {
        nodata: Some(0),
        classes,
    }
}

/// Search the usual places for a raster/shapefile folder.
pub fn raster_search_dirs(explicit: Option<&Path>, data_folder: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(p) = explicit {
        dirs.push(p.to_path_buf());
    }
    if let Ok(env) = std::env::var("FOREST_DATA_COMPILER_RASTERS") {
        let p = PathBuf::from(env);
        if !dirs.iter().any(|d| d == &p) {
            dirs.push(p);
        }
    }
    if let Some(data) = data_folder {
        let nested = data.join("rasters");
        if !dirs.iter().any(|d| d == &nested) {
            dirs.push(nested);
        }
    }
    dirs
}

#[cfg(test)]
pub(crate) fn write_test_gez_geotiff(path: &Path, width: u32, height: u32, origin_lon: f64, origin_lat: f64, scale: f64, fill: u8) {
    let pixels = vec![fill; (width * height) as usize];
    write_classified_geotiff(path, width, height, origin_lon, origin_lat, scale, &pixels);
}

#[cfg(test)]
pub(crate) fn write_test_gez_shapefile(path: &Path) {
    write_test_labeled_shapefile(path, "gez_name", "Tropical rain forest");
}

#[cfg(test)]
pub(crate) fn write_test_labeled_shapefile(path: &Path, field: &str, value: &str) {
    use shapefile::dbase::{FieldValue, Record, TableWriterBuilder};
    use shapefile::{Point, Polygon, PolygonRing, Writer};
    use std::convert::TryFrom;

    let builder = TableWriterBuilder::new()
        .add_character_field(TryFrom::try_from(field).unwrap(), 40);
    let mut writer = Writer::from_path(path, builder).unwrap();
    let ring = PolygonRing::Outer(vec![
        Point::new(-62.0, 0.0),
        Point::new(-58.0, 0.0),
        Point::new(-58.0, -4.0),
        Point::new(-62.0, -4.0),
        Point::new(-62.0, 0.0),
    ]);
    let poly = Polygon::new(ring);
    let mut rec = Record::default();
    rec.insert(field.into(), FieldValue::Character(Some(value.into())));
    writer.write_shape_and_record(&poly, &rec).unwrap();
}

#[cfg(test)]
fn write_classified_geotiff(
    path: &Path,
    width: u32,
    height: u32,
    origin_lon: f64,
    origin_lat: f64,
    scale: f64,
    pixels: &[u8],
) {
    assert_eq!(pixels.len(), (width * height) as usize);
    let mut buf = Vec::new();
    let n_tags = 12u16;
    let header = 8u32;
    let ifd_len = 2 + u32::from(n_tags) * 12 + 4;
    let pixels_off = header + ifd_len;
    let scale_off = pixels_off + pixels.len() as u32;
    let tie_off = scale_off + 24;
    let geo_off = tie_off + 48;

    buf.extend_from_slice(&[0x49, 0x49, 0x2A, 0x00]);
    buf.extend_from_slice(&header.to_le_bytes());
    buf.extend_from_slice(&n_tags.to_le_bytes());

    let short = |b: &mut Vec<u8>, tag: u16, value: u32| {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&value.to_le_bytes());
    };
    let long = |b: &mut Vec<u8>, tag: u16, value: u32| {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&value.to_le_bytes());
    };
    let doubles = |b: &mut Vec<u8>, tag: u16, count: u32, offset: u32| {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&12u16.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
    };

    short(&mut buf, 256, width);
    short(&mut buf, 257, height);
    short(&mut buf, 258, 8);
    short(&mut buf, 259, 1);
    short(&mut buf, 262, 1);
    long(&mut buf, 273, pixels_off);
    short(&mut buf, 277, 1);
    short(&mut buf, 278, height);
    long(&mut buf, 279, pixels.len() as u32);
    doubles(&mut buf, 33550, 3, scale_off);
    doubles(&mut buf, 33922, 6, tie_off);
    buf.extend_from_slice(&34735u16.to_le_bytes());
    buf.extend_from_slice(&3u16.to_le_bytes());
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&geo_off.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());

    buf.extend_from_slice(pixels);
    for v in [scale, scale, 0.0_f64] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0.0_f64, 0.0, 0.0, origin_lon, origin_lat, 0.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let keys: [u16; 16] = [
        1, 1, 0, 3,
        1024, 0, 1, 2,
        1025, 0, 1, 1,
        2048, 0, 1, 4326,
    ];
    for k in keys {
        buf.extend_from_slice(&k.to_le_bytes());
    }
    std::fs::write(path, buf).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fao_legend_maps_tropical_rain_forest() {
        let legend = fao_gez_legend();
        let class = legend.lookup(1).unwrap();
        assert_eq!(class.bioregion.as_deref(), Some("Tropical moist broadleaf forests"));
        assert!(class.forest_types.iter().any(|f| f == "Tropical"));
        assert_eq!(
            legend.lookup_token("TAr").unwrap().label,
            "Tropical rain forest"
        );
    }

    #[test]
    fn samples_amazon_cell_from_test_geotiff() {
        let dir = std::env::temp_dir().join("fdc-raster-gez");
        let _ = std::fs::create_dir_all(&dir);
        let tif = dir.join("fao_gez2010.tif");
        write_test_gez_geotiff(&tif, 4, 4, -62.0, 0.0, 1.0, 1);
        let catalog = RasterCatalog::from_dir(&dir).expect("catalog");
        let suggestion = catalog.suggest(&[(-2.5, -60.0)]);
        assert!(
            suggestion.ecoregions.iter().any(|e| e == "Tropical moist broadleaf forests"),
            "{suggestion:?}"
        );
        assert!(suggestion.forest_types.iter().any(|f| f == "Tropical"));
        let _ = std::fs::remove_file(&tif);
    }

    #[test]
    fn samples_amazon_from_test_shapefile() {
        let dir = std::env::temp_dir().join("fdc-shape-gez");
        let _ = std::fs::create_dir_all(&dir);
        let shp = dir.join("fao_gez2010.shp");
        write_test_gez_shapefile(&shp);
        let catalog = RasterCatalog::from_dir(&dir).expect("catalog");
        assert_eq!(catalog.ecoregion.as_ref().unwrap().format_name(), "shapefile");
        let suggestion = catalog.suggest(&[(-2.5, -60.0)]);
        assert!(
            suggestion.ecoregions.iter().any(|e| e == "Tropical moist broadleaf forests"),
            "{suggestion:?}"
        );
        assert!(suggestion.forest_types.iter().any(|f| f == "Tropical"));
        for ext in ["shp", "shx", "dbf"] {
            let _ = std::fs::remove_file(shp.with_extension(ext));
        }
    }

    #[test]
    fn forest_type_layer_adds_to_gez_types() {
        let dir = std::env::temp_dir().join("fdc-shape-both");
        let _ = std::fs::create_dir_all(&dir);
        let gez = dir.join("fao_gez2010.shp");
        let fty = dir.join("mangrove_forest_type.shp");
        write_test_gez_shapefile(&gez);
        write_test_labeled_shapefile(&fty, "ForestType", "Mangrove");
        let catalog = RasterCatalog::load(
            &LayerSources {
                ecoregion: Some(gez.clone()),
                forest_type: Some(fty.clone()),
                folder: None,
            },
            None,
        )
        .expect("catalog");
        let suggestion = catalog.suggest(&[(-2.5, -60.0)]);
        assert!(suggestion.ecoregions.iter().any(|e| e == "Tropical moist broadleaf forests"));
        assert!(suggestion.forest_types.iter().any(|f| f == "Tropical"));
        assert!(suggestion.forest_types.iter().any(|f| f == "Mangrove"), "{suggestion:?}");
        assert_eq!(suggestion.forest_type_source.as_deref(), Some("combined"));
        for path in [&gez, &fty] {
            for ext in ["shp", "shx", "dbf"] {
                let _ = std::fs::remove_file(path.with_extension(ext));
            }
        }
    }
}
