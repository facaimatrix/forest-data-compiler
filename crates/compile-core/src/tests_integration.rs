#[cfg(test)]
mod tests {
    use crate::compile::{compile_files, CompileFormat, CompileOptions};
    use crate::manifest::{CompileManifest, ManifestScope};
    use crate::match_files::{match_folder, MatchOptions};
    use crate::reader::peek_column_uniques;
    use std::path::{Path, PathBuf};

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }

    fn dummy_root() -> PathBuf {
        fixtures_root().join("dummy")
    }

    fn dummy_manifest(name: &str) -> CompileManifest {
        CompileManifest::from_path(&dummy_root().join("sample-manifests").join(name)).unwrap()
    }

    fn uniques(path: &Path, columns: &[&str]) -> Vec<String> {
        peek_column_uniques(path, columns, 100_000).unwrap()
    }

    fn cleanup(out: &Path) {
        let _ = std::fs::remove_file(out);
        let _ = std::fs::remove_file(format!("{}.compile-report.json", out.display()));
    }

    #[test]
    fn loads_sample_manifest_and_matches() {
        let root = fixtures_root();
        let manifest = CompileManifest::from_path(&root.join("sample-manifest.json")).unwrap();
        assert_eq!(manifest.project.title, "Sample Amazon Height Study");

        let candidates = match_folder(
            &root.join("data"),
            &manifest,
            &MatchOptions {
                joined_owners_only: true,
                include_unregistered: false,
                recursive: false,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(candidates.candidates.len(), 1);
        assert_eq!(candidates.candidates[0].file_name, "Amazon_PlotA_GFB3.csv");
        assert!(candidates.candidates[0].selected);
    }

    #[test]
    fn compiles_csv_merge() {
        let root = fixtures_root();
        let manifest = CompileManifest::from_path(&root.join("sample-manifest.json")).unwrap();
        let out = std::env::temp_dir().join("forest-data-compiler-test.csv");
        let paths = vec![
            root.join("data/Amazon_PlotA_GFB3.csv"),
            root.join("data/Amazon_PlotB_GFB3.csv"),
        ];
        let report = compile_files(
            &paths,
            &out,
            &manifest,
            &CompileOptions {
                format: CompileFormat::Csv,
                add_source_column: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.source_count, 2);
        assert_eq!(report.total_rows, 5);
        assert!(out.exists());
        cleanup(&out);
    }

    #[test]
    fn accepts_current_and_legacy_schema_ids() {
        let text = std::fs::read_to_string(fixtures_root().join("sample-manifest.json")).unwrap();
        assert!(text.contains("forest-data-exchange.compile_manifest"));
        CompileManifest::from_json(&text).unwrap();

        let legacy = text.replace(
            "forest-data-exchange.compile_manifest",
            "internodes.compile_manifest",
        );
        CompileManifest::from_json(&legacy).unwrap();

        let bogus = text.replace("forest-data-exchange.compile_manifest", "some.other.schema");
        assert!(CompileManifest::from_json(&bogus).is_err());
    }

    #[test]
    fn reads_dummy_manifest_requirements() {
        let manifest =
            CompileManifest::from_path(&dummy_root().join(
                "forest-data-exchange-compile-manifest-dummy.json",
            ))
            .unwrap();

        assert_eq!(manifest.scope(), ManifestScope::Countries);
        assert_eq!(
            manifest.requirements.geography.countries,
            vec!["Brazil", "Indonesia", "Malaysia"]
        );
        assert_eq!(manifest.forest_type_filter(), vec!["Tropical", "Mangrove"]);
        // "present" is an open-ended upper bound.
        assert_eq!(manifest.year_bounds(), (Some(1950), None));
        assert_eq!(manifest.requirements.mandatory_map_products.len(), 1);
    }

    #[test]
    fn country_and_forest_type_filters_apply_to_rows() {
        let manifest = dummy_manifest("countries-brazil-indonesia.json");
        let out = std::env::temp_dir().join("fdc-test-countries.csv");
        let report = compile_files(
            &[dummy_root().join("DummyNetwork_GFB3.csv")],
            &out,
            &manifest,
            &CompileOptions {
                format: CompileFormat::Csv,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.total_rows > 0);
        let mut countries = uniques(&out, &["Country"]);
        countries.sort();
        assert_eq!(countries, vec!["Brazil", "Indonesia"]);
        cleanup(&out);
    }

    #[test]
    fn year_range_filter_respects_closed_upper_bound() {
        let manifest = dummy_manifest("pre-1950.json");
        assert_eq!(manifest.year_bounds(), (Some(1900), Some(1949)));

        let out = std::env::temp_dir().join("fdc-test-pre1950.csv");
        let report = compile_files(
            &[dummy_root().join("DummyNetwork_GFB3.csv")],
            &out,
            &manifest,
            &CompileOptions {
                format: CompileFormat::Csv,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.total_rows > 0);
        for year in uniques(&out, &["YR"]) {
            let y: i64 = year.trim().parse().unwrap();
            assert!((1900..=1949).contains(&y), "unexpected year {y}");
        }
        cleanup(&out);
    }

    #[test]
    fn infers_and_writes_dataset_metadata_sidecar() {
        use crate::dataset_metadata::{
            inspect_file, sidecar_path, write_sidecars, DatasetMetadata, SCHEMA, WriteOptions,
        };

        let csv = dummy_root().join("DummyNetwork_GFB3.csv");
        let inspect = inspect_file(&csv).unwrap();
        let meta = &inspect.metadata;
        assert_eq!(meta.schema, SCHEMA);
        assert!(meta.source.looks_like_gfb3);
        assert_eq!(meta.attributes.get("tree_height"), Some(&true));
        assert_eq!(meta.attributes.get("agb"), Some(&true));
        assert!(meta.geography.countries.iter().any(|c| c == "Brazil"));
        assert!(meta.geography.countries.iter().any(|c| c == "Indonesia"));
        assert!(meta.forest_types.iter().any(|f| f == "Tropical"));
        assert!(meta.forest_types.iter().any(|f| f == "Mangrove"));
        let years = meta.year_range.as_ref().expect("year range");
        assert_eq!(years.year_start, Some(1930));
        assert_eq!(years.year_end, Some(2024));
        assert_eq!(meta.num_plots, Some(518));

        let dest = std::env::temp_dir().join("fdc-meta-DummyNetwork_GFB3.csv");
        std::fs::write(&dest, std::fs::read(&csv).unwrap()).unwrap();
        let mut copy = meta.clone();
        copy.source.path = dest.display().to_string();
        copy.source.file_name = "fdc-meta-DummyNetwork_GFB3.csv".into();
        copy.coauthors = vec![crate::dataset_metadata::Coauthor {
            author_name: "Ada Rivera".into(),
            author_email: Some("ada@example.org".into()),
            role: "co_author".into(),
            author_order: Some(2),
            affiliation: Some("Example Lab".into()),
        }];
        let report = write_sidecars(
            &[copy],
            &WriteOptions {
                overwrite: true,
                contributor_email: Some("owner1@example.org".into()),
                contributor_name: Some("Dummy Network".into()),
            },
        )
        .unwrap();
        assert_eq!(report.written.len(), 1);
        let loaded = DatasetMetadata::from_path(&sidecar_path(&dest)).unwrap();
        assert_eq!(loaded.contributor_email.as_deref(), Some("owner1@example.org"));
        assert!(loaded.geography.countries.contains(&"Brazil".to_string()));
        assert_eq!(loaded.coauthors.len(), 2);
        assert_eq!(loaded.coauthors[0].author_name, "Dummy Network");
        assert_eq!(loaded.coauthors[0].role, "corresponding");
        assert_eq!(loaded.coauthors[1].author_name, "Ada Rivera");
        assert_eq!(loaded.coauthors[1].author_email.as_deref(), Some("ada@example.org"));

        assert!(
            !loaded.notes.iter().any(|n| n.contains("by hand")),
            "filled metadata should not keep 'fill by hand' notes: {:?}",
            loaded.notes
        );

        let reloaded = inspect_file(&dest).unwrap().metadata;
        assert_eq!(reloaded.coauthors.len(), 2);
        assert_eq!(reloaded.coauthors[1].affiliation.as_deref(), Some("Example Lab"));

        let directory = crate::dataset_metadata::build_author_directory(&[reloaded.clone()]);
        assert_eq!(directory.people.len(), 2);
        let authors_report = crate::dataset_metadata::write_author_sidecars(&[reloaded]).unwrap();
        assert_eq!(authors_report.written.len(), 1);
        let authors = std::path::PathBuf::from(&authors_report.written[0]);
        assert_eq!(
            authors.file_name().unwrap().to_string_lossy(),
            "fdc-meta-DummyNetwork_GFB3_authors.json"
        );
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(sidecar_path(&dest));
        let _ = std::fs::remove_file(&authors);
    }

    #[test]
    fn inspect_fills_ecoregion_from_fao_gez_raster() {
        use crate::dataset_metadata::inspect_file_with;
        use crate::raster_lookup::{write_test_gez_geotiff, RasterCatalog};

        let dir = std::env::temp_dir().join("fdc-nogeo-raster");
        let _ = std::fs::create_dir_all(&dir);
        let csv = dir.join("AmazonPlot_GFB3.csv");
        let tif = dir.join("fao_gez2010.tif");
        std::fs::write(
            &csv,
            "PlotID,TreeID,YR,Latitude,Longitude\nP1,T1,2010,-2.5,-60.0\n",
        )
        .unwrap();
        write_test_gez_geotiff(&tif, 4, 4, -62.0, 0.0, 1.0, 1);
        let catalog = RasterCatalog::from_dir(&dir).unwrap();
        let inspect = inspect_file_with(&csv, Some(&catalog)).unwrap();
        assert_eq!(
            inspect.metadata.geography.ecoregion_source.as_deref(),
            Some("geotiff")
        );
        assert!(inspect
            .metadata
            .geography
            .ecoregions
            .iter()
            .any(|e| e == "Tropical moist broadleaf forests"));
        assert!(inspect.metadata.forest_types.iter().any(|f| f == "Tropical"));
        let _ = std::fs::remove_file(&csv);
        let _ = std::fs::remove_file(&tif);
    }

    #[test]
    fn inspect_fills_ecoregion_from_fao_gez_shapefile() {
        use crate::dataset_metadata::inspect_file_with;
        use crate::raster_lookup::{write_test_gez_shapefile, RasterCatalog};

        let dir = std::env::temp_dir().join("fdc-nogeo-shp");
        let _ = std::fs::create_dir_all(&dir);
        let csv = dir.join("AmazonPlot_GFB3.csv");
        let shp = dir.join("fao_gez2010.shp");
        std::fs::write(
            &csv,
            "PlotID,TreeID,YR,Latitude,Longitude\nP1,T1,2010,-2.5,-60.0\n",
        )
        .unwrap();
        write_test_gez_shapefile(&shp);
        let catalog = RasterCatalog::from_dir(&dir).unwrap();
        let inspect = inspect_file_with(&csv, Some(&catalog)).unwrap();
        assert_eq!(
            inspect.metadata.geography.ecoregion_source.as_deref(),
            Some("shapefile")
        );
        assert!(inspect
            .metadata
            .geography
            .ecoregions
            .iter()
            .any(|e| e == "Tropical moist broadleaf forests"));
        let _ = std::fs::remove_file(&csv);
        for ext in ["shp", "shx", "dbf"] {
            let _ = std::fs::remove_file(shp.with_extension(ext));
        }
    }

    #[test]
    fn inspect_suggests_country_when_table_has_no_country_column() {
        use crate::dataset_metadata::{authors_path, inspect_file};

        let dest = std::env::temp_dir().join("fdc-nogeo-AmazonPlot_GFB3.csv");
        std::fs::write(
            &dest,
            "PlotID,TreeID,YR,Latitude,Longitude\nP1,T1,2010,-2.5,-60.0\n",
        )
        .unwrap();
        let inspect = inspect_file(&dest).unwrap();
        assert!(inspect.suggested_countries.iter().any(|c| c == "Brazil"));
        assert!(inspect.metadata.geography.countries.iter().any(|c| c == "Brazil"));
        assert_eq!(
            inspect.metadata.geography.country_source.as_deref(),
            Some("coordinates")
        );
        assert_eq!(
            authors_path(&dest).file_name().unwrap().to_string_lossy(),
            "fdc-nogeo-AmazonPlot_GFB3_authors.json"
        );
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn suggests_country_from_plot_coordinates() {
        use crate::country_lookup::suggest_from_coordinates;
        use crate::reader::peek_coordinate_pairs;

        let points = peek_coordinate_pairs(
            &dummy_root().join("DummyNetwork_GFB3.csv"),
            50_000,
        )
        .unwrap();
        assert!(!points.is_empty());
        let (countries, continents) = suggest_from_coordinates(&points);
        assert!(countries.iter().any(|c| c == "Brazil"));
        assert!(continents.iter().any(|c| c == "South America"));
    }

    #[test]
    fn absent_forest_type_compiles_to_nothing() {
        let manifest = dummy_manifest("plantation-empty.json");
        let out = std::env::temp_dir().join("fdc-test-plantation.csv");
        let err = compile_files(
            &[dummy_root().join("DummyNetwork_GFB3.csv")],
            &out,
            &manifest,
            &CompileOptions {
                format: CompileFormat::Csv,
                ..Default::default()
            },
        )
        .unwrap_err();

        assert!(err.contains("No rows left"), "unexpected error: {err}");
        cleanup(&out);
    }

    #[test]
    fn shapefile_extent_selects_amazon_dataset_names() {
        use crate::geo::{GeoFilter, GeoMode};
        use crate::raster_lookup::write_test_gez_shapefile;

        let dir = std::env::temp_dir().join(format!("fdc-extent-select-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let amazon = dir.join("AmazonInside_GFB3.csv");
        let paris = dir.join("ParisOutside_GFB3.csv");
        let shp = dir.join("extent.shp");
        let header = "PlotID,TreeID,YR,Status,DBH,Species,Latitude,Longitude,Country\n";
        std::fs::write(
            &amazon,
            format!("{header}P1,T1,2010,0,12.0,Ocotea,-2.5,-60.0,Brazil\n"),
        )
        .unwrap();
        std::fs::write(
            &paris,
            format!("{header}P2,T1,2010,0,11.0,Quercus,48.85,2.35,France\n"),
        )
        .unwrap();
        write_test_gez_shapefile(&shp);

        let manifest = dummy_manifest("global-all.json");
        let bundle = match_folder(
            &dir,
            &manifest,
            &MatchOptions {
                joined_owners_only: false,
                include_unregistered: true,
                recursive: false,
                geo: GeoFilter {
                    mode: GeoMode::Shapefile,
                    shapefile_path: Some(shp.display().to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap();

        let names: Vec<_> = bundle
            .candidates
            .iter()
            .filter(|c| c.selected)
            .map(|c| c.file_name.as_str())
            .collect();
        assert_eq!(names, vec!["AmazonInside_GFB3.csv"]);
        let amazon_hit = bundle
            .candidates
            .iter()
            .find(|c| c.file_name == "AmazonInside_GFB3.csv")
            .unwrap();
        assert_eq!(amazon_hit.plots_in_extent, Some(1));

        let out = std::env::temp_dir().join("fdc-extent-compile.csv");
        let report = compile_files(
            &[amazon, paris],
            &out,
            &manifest,
            &CompileOptions {
                format: CompileFormat::Csv,
                add_source_column: true,
                geo: GeoFilter {
                    mode: GeoMode::Shapefile,
                    shapefile_path: Some(shp.display().to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert_eq!(report.source_count, 1);
        assert_eq!(report.sources[0].file_name, "AmazonInside_GFB3.csv");
        assert_eq!(report.total_rows, 1);
        cleanup(&out);
    }
}
