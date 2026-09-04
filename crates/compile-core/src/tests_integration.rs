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

        let reloaded = inspect_file(&dest).unwrap().metadata;
        assert_eq!(reloaded.coauthors.len(), 2);
        assert_eq!(reloaded.coauthors[1].affiliation.as_deref(), Some("Example Lab"));

        let directory = crate::dataset_metadata::build_author_directory(&[reloaded]);
        assert_eq!(directory.people.len(), 2);
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(sidecar_path(&dest));
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
}
