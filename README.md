# Forest Data Compiler

Desktop app for Forest Data Exchange admins: load a project **compile manifest** JSON, pick a local folder of GFB3 tree-level files, filter matches, and compile a single deliverable for manual upload back to Forest Data Exchange.

## Flow

1. In Forest Data Exchange **Admin → Compiled Data**, download the requirements JSON for an approved project (`forest-data-exchange-compile-manifest-<project>.json`).
2. Open that JSON here.
3. Choose the local folder that holds contributor GFB3 files (CSV / TSV / XLSX / Parquet).
4. Review matches (registered dataset names + mandatory attributes + geography + forest types).
5. Compile → save CSV / XLSX / Parquet / ZIP.
6. Upload the compiled file in Forest Data Exchange **Admin → Compiled Data → Manual upload**.

Output files are named `forest-data-exchange-compiled-<project>-<id>-<date>.<ext>`, with a
`.compile-report.json` sidecar listing sources, row counts and every filter that was applied.

## Run (dev)

```powershell
.\dev.ps1
```

Requires Rust + [Tauri CLI v2](https://v2.tauri.app/):

```powershell
cargo install tauri-cli --version "^2"
```

## Stack

- Rust + Tauri v2 (same family as Forest Data Harmonizer)
- `compile-core`: manifest parsing, folder matching, Polars merge
- Static web UI in `apps/compiler/web`

## Manifest schema

Expects `schema: "forest-data-exchange.compile_manifest"` / `version: 1` as produced by the
website's `src/lib/compileManifest.js`. The pre-rename id `internodes.compile_manifest` is still
accepted so older exports keep working.

### Requirements the compiler enforces

| Requirement | Where it comes from | How it is applied |
| --- | --- | --- |
| Mandatory attributes | `requirements.mandatory_attributes` | File-level screen; skipped when `matching_rules.dataset_must_have_all_mandatory_attributes` is false |
| Geography | `requirements.geography` (`global`, `continental`, `ecoregion`, `countries`) | File-level screen plus row filter on Country / Continent / Bioregion columns |
| Forest types | `requirements.forest_types` (`All` = no filter) | File-level screen plus row filter on the ForestType column |
| Census years | `requirements.year_range` (`year_end: "present"` = open ended) | Row filter on the YR / Year column |

Custom extent shapefiles and map products are reported in the UI but not enforced: the compiler
does no GIS work, so use the country or bioregion filter for shapefile-scoped projects.

## Dataset metadata (legacy ingest)

GFB3 files that entered the archive before Forest Data Exchange have no website
`data_files` record and no sidecar. In the app, switch to **Dataset metadata**,
pick the folder, and write `{stem}.metadata.json` next to each table.

Each sidecar uses `schema: "forest-data-exchange.dataset_metadata"` and records
the same fields the website stores on a dataset: attribute flags, countries /
continents / ecoregions, forest types, census year window, and plot / tree
counts. Use **Edit metadata** to fill contributor, countries, continents, bioregions,
forest types, census years, attributes, and coauthors. If the table has plot
coordinates but no Country column, countries (and continents) are suggested
from those coordinates — review plots near borders.

To auto-fill **bioregion** and **forest type**, pick a layer per field — shapefile
(`.shp`) or GeoTIFF (`.tif`). FAO GEZ 2010 as a **shapefile** is the better
ecoregion source; a classified forest-type GeoTIFF is fine for the other field.
You can also drop files in `{data folder}/rasters` or set
`FOREST_DATA_COMPILER_RASTERS`. Layers must be geographic WGS84 (EPSG:4326).
Optional `{stem}.legend.json` maps pixel codes or shapefile attributes; FAO GEZ
names (`TAr`, `Tropical rain forest`, …) are recognized. The app does not ship
the FAO files; download GEZ from the
[FAO catalog](https://data.apps.fao.org/catalog/dataset/2fb209d0-fd34-4e5e-a3d8-a13c241eb61b).

Each sidecar also holds a `coauthors` list (`author_name`, `author_email`,
`affiliation`, `role`, `author_order`). Roles match the website
(`corresponding` / `co_author`). **Write {dataset}_authors.json** writes a
de-duplicated contact list next to each selected file as `{stem}_authors.json`.

The compile matcher reads these sidecars so a backfilled file can be selected
even when it is missing from a project manifest's `registered_datasets`.

## Fixtures

- `fixtures/sample-manifest.json` + `fixtures/data/` — smoke tests.
- `fixtures/dummy/` — dummy network (518 plots, `DummyNetwork_GFB3.csv`) with `coverage.json`
  describing what is in it, plus `sample-manifests/` covering global, continental, country,
  forest-type and year-range scopes (`plantation-empty.json` is expected to match nothing).
