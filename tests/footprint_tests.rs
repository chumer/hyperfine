mod common;

use common::hyperfine;
use predicates::prelude::*;

#[test]
fn footprint_rejects_unsupported_configuration_before_setup_or_export() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("setup-ran");
    let export = directory.path().join("results.json");
    std::fs::write(&export, "previous contents").unwrap();
    hyperfine()
        .args([
            "--metrics=memory_peak_footprint",
            if cfg!(target_os = "macos") {
                "--shell=default"
            } else {
                "--shell=none"
            },
            "--setup",
        ])
        .arg(format!("echo touched > \"{}\"", marker.display()))
        .arg("--export-json")
        .arg(&export)
        .arg("echo benchmark")
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("memory_peak_footprint"));
    assert!(!marker.exists());
    assert_eq!(
        std::fs::read_to_string(export).unwrap(),
        "previous contents"
    );
}

#[test]
fn missing_footprint_is_not_replaced_with_resident_memory_or_zero() {
    // The mock executor supplies RSS but no footprint.
    hyperfine()
        .args([
            "--debug-mode",
            "--style=none",
            "--runs=1",
            "--metrics=memory_peak_footprint",
            "sleep 1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Metric 'memory_peak_footprint' is unavailable",
        ));
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    #[test]
    fn footprint_tracks_private_lifetime_peaks_not_clean_files_or_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("footprint-helper");
        let compilation = Command::new("cc")
            .args([
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "tests/fixtures/footprint.c",
                "-o",
            ])
            .arg(&helper)
            .output()
            .unwrap();
        assert!(compilation.status.success(), "{:?}", compilation);

        const SIZE: usize = 128 * 1024 * 1024;
        let file_path = directory.path().join("mapped.bin");
        let mut file = std::fs::File::create(&file_path).unwrap();
        let block = [42_u8; 64 * 1024];
        for _ in 0..SIZE / block.len() {
            file.write_all(&block).unwrap();
        }
        drop(file);

        let json_path = directory.path().join("results.json");
        let csv_path = directory.path().join("results.csv");
        let markdown_path = directory.path().join("results.md");
        let asciidoc_path = directory.path().join("results.adoc");
        let org_path = directory.path().join("results.org");
        let modes = ["none", "alloc", "free", "read", "cow", "child"];
        let mut benchmark = hyperfine();
        benchmark.args([
            "--style=none",
            "--runs=3",
            "--metrics=memory_peak_footprint:MiB",
            "--export-json",
        ]);
        benchmark
            .arg(&json_path)
            .arg("--export-csv")
            .arg(&csv_path)
            .arg("--export-markdown")
            .arg(&markdown_path)
            .arg("--export-asciidoc")
            .arg(&asciidoc_path)
            .arg("--export-orgmode")
            .arg(&org_path);
        for mode in modes {
            benchmark.arg(shell_words::join([
                helper.to_str().unwrap(),
                mode,
                &SIZE.to_string(),
                file_path.to_str().unwrap(),
            ]));
        }
        benchmark.assert().success();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
        assert_eq!(json["primary_metric"], "memory_peak_footprint");
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), modes.len());
        let mut peaks = Vec::new();
        for result in results {
            let summary = &result["summary"]["memory_peak_footprint"];
            assert_eq!(summary["unit"], "byte");
            assert_eq!(summary["count"], 3);
            peaks.push(summary["mean"].as_f64().unwrap());
            for sample in result["measurements"].as_array().unwrap() {
                assert_eq!(sample["memory_peak_footprint"]["unit"], "byte");
                assert!(sample["memory_peak_footprint"]["value"].as_f64().unwrap() > 0.0);
                assert!(sample.get("memory_peak_resident").is_some());
                assert_eq!(sample["exit_code"], 0);
            }
        }
        // Large allocations and broad tolerances avoid exact OS-accounting thresholds.
        for index in [1, 2, 4] {
            let increase = peaks[index] - peaks[0];
            assert!(
                (0.75 * SIZE as f64..1.25 * SIZE as f64).contains(&increase),
                "{:?}",
                peaks
            );
        }
        for index in [3, 5] {
            assert!(peaks[index] - peaks[0] < 0.25 * SIZE as f64, "{:?}", peaks);
        }
        assert!(peaks[4] - peaks[3] > 0.75 * SIZE as f64, "{:?}", peaks);

        let mut csv = csv::Reader::from_path(csv_path).unwrap();
        let headers = csv.headers().unwrap().clone();
        let unit_column = headers
            .iter()
            .position(|h| h == "memory_peak_footprint_unit")
            .unwrap();
        let mean_column = headers
            .iter()
            .position(|h| h == "memory_peak_footprint_mean")
            .unwrap();
        let rows = csv.records().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), modes.len());
        for (row, peak) in rows.iter().zip(peaks) {
            assert_eq!(&row[unit_column], "MiB");
            let mean: f64 = row[mean_column].parse().unwrap();
            approx::assert_relative_eq!(mean, peak / 1_048_576.0);
        }
        // Markup is human-readable: only exercise serialization, not exact formatting.
        for path in [markdown_path, asciidoc_path, org_path] {
            assert!(!std::fs::read(path).unwrap().is_empty());
        }
    }

    #[test]
    fn footprint_is_collected_independently_of_display_and_hardware_counters() {
        for metrics in ["default", "all", "memory_peak_footprint"] {
            let directory = tempfile::tempdir().unwrap();
            let export = directory.path().join("results.json");
            hyperfine()
                .args(["--style=none", "--runs=1", "--warmup=1", "--metrics"])
                .arg(metrics)
                .arg("--export-json")
                .arg(&export)
                .arg("true")
                .assert()
                .success();
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
            let result = &json["results"][0];
            assert_eq!(result["measurements"].as_array().unwrap().len(), 1);
            assert_eq!(result["summary"]["memory_peak_footprint"]["unit"], "byte");
            assert!(
                result["measurements"][0]["memory_peak_footprint"]["value"]
                    .as_f64()
                    .unwrap()
                    > 0.0
            );
        }
        let directory = tempfile::tempdir().unwrap();
        let export = directory.path().join("shell.json");
        hyperfine()
            .args([
                "--style=none",
                "--runs=1",
                "--shell=default",
                "--metrics=all",
                "--export-json",
            ])
            .arg(&export)
            .arg("true")
            .assert()
            .success();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
        assert!(json["results"][0]["summary"]
            .get("memory_peak_footprint")
            .is_none());
        assert!(json["results"][0]["measurements"][0]
            .get("memory_peak_footprint")
            .is_none());
    }
}
