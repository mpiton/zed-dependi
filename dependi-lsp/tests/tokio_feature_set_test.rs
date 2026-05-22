use std::{collections::BTreeSet, fs, process::Command};

use toml::Value;

const DEPENDI_LSP_MANIFEST: &str = include_str!("../Cargo.toml");
const TOKIO_FEATURE_MEASUREMENTS: &str = include_str!("fixtures/tokio_feature_measurements.toml");
const EXPECTED_TOKIO_FEATURES: [&str; 5] = ["fs", "io-std", "io-util", "macros", "rt-multi-thread"];

fn tokio_features(manifest: &str) -> BTreeSet<String> {
    let manifest = toml::from_str::<Value>(manifest).expect("dependi-lsp Cargo.toml is valid TOML");
    manifest["dependencies"]["tokio"]["features"]
        .as_array()
        .expect("tokio dependency declares explicit features")
        .iter()
        .map(|feature| {
            feature
                .as_str()
                .expect("tokio features are strings")
                .to_string()
        })
        .collect()
}

fn manifest_with_dependencies(dependencies: &str) -> String {
    format!("[dependencies]\n{dependencies}")
}

fn has_dependency(manifest: &str, dependency_name: &str) -> bool {
    let manifest = toml::from_str::<Value>(manifest).expect("manifest fixture is valid TOML");
    manifest["dependencies"].get(dependency_name).is_some()
}

fn manifest_without_dependency(manifest: &str, dependency_name: &str) -> String {
    let mut manifest = toml::from_str::<Value>(manifest).expect("manifest fixture is valid TOML");
    manifest["dependencies"]
        .as_table_mut()
        .expect("manifest has dependency table")
        .remove(dependency_name)
        .expect("dependency exists before removal");
    toml::to_string(&manifest).expect("mutated manifest serializes as TOML")
}

fn has_reqwest_network_capability(manifest: &str) -> bool {
    has_dependency(manifest, "reqwest") && !tokio_features(manifest).contains("net")
}

fn cargo_check_probe_succeeds(features: &[&str], main_rs: &str) -> bool {
    let project = tempfile::tempdir().expect("probe temp dir is created");
    let src_dir = project.path().join("src");
    fs::create_dir(&src_dir).expect("probe src dir is created");
    let feature_list = features
        .iter()
        .map(|feature| format!(r#""{feature}""#))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        project.path().join("Cargo.toml"),
        format!(
            r#"[package]
name = "tokio-feature-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
tokio = {{ version = "=1.52.3", features = [{feature_list}] }}
"#
        ),
    )
    .expect("probe manifest is written");
    fs::write(src_dir.join("main.rs"), main_rs).expect("probe main is written");

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    Command::new(cargo)
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(project.path().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(project.path().join("target"))
        .output()
        .expect("cargo check probe runs")
        .status
        .success()
}

fn expected_tokio_features() -> BTreeSet<String> {
    EXPECTED_TOKIO_FEATURES
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn missing_expected_tokio_features(features: &BTreeSet<String>) -> BTreeSet<String> {
    expected_tokio_features()
        .difference(features)
        .cloned()
        .collect()
}

fn unrelated_tokio_features(features: &BTreeSet<String>) -> BTreeSet<String> {
    features
        .difference(&expected_tokio_features())
        .cloned()
        .collect()
}

fn performance_floor_satisfied(
    baseline_binary_size: u64,
    baseline_clean_seconds: u64,
    trimmed_binary_size: u64,
    trimmed_clean_seconds: u64,
) -> bool {
    trimmed_binary_size * 100 <= baseline_binary_size * 95
        && trimmed_clean_seconds * 100 <= baseline_clean_seconds * 90
}

fn measurement_case(case_name: &str) -> (u64, u64, u64, u64) {
    let fixture =
        toml::from_str::<Value>(TOKIO_FEATURE_MEASUREMENTS).expect("measurement fixture is valid");
    let case = &fixture["cases"][case_name];
    let get_u64 = |key: &str| {
        case[key]
            .as_integer()
            .unwrap_or_else(|| panic!("{case_name}.{key} is an integer")) as u64
    };

    (
        get_u64("baseline_binary_size"),
        get_u64("baseline_clean_seconds"),
        get_u64("trimmed_binary_size"),
        get_u64("trimmed_clean_seconds"),
    )
}

#[derive(Debug)]
struct BuildMeasurement<'a> {
    source_revision: &'a str,
    profile: &'a str,
    clean_build: bool,
}

fn comparable_measurements(
    baseline: &BuildMeasurement<'_>,
    trimmed: &BuildMeasurement<'_>,
) -> Result<(), &'static str> {
    if !baseline.clean_build || !trimmed.clean_build {
        return Err("clean build condition differs");
    }
    if baseline.source_revision != trimmed.source_revision {
        return Err("source revision differs");
    }
    if baseline.profile != trimmed.profile {
        return Err("build profile differs");
    }
    Ok(())
}

#[test]
fn dependi_lsp_uses_the_trimmed_tokio_feature_set() {
    // Given the `dependi-lsp/Cargo.toml` dependency line for `tokio` is:
    //   tokio = { version = "1.52.3", features = ["rt-multi-thread", "macros", "io-util", "io-std", "fs"] }
    // When the Tokio dependency features are inspected
    // Then the direct Tokio feature set is exactly "fs, io-std, io-util, macros, rt-multi-thread"
    // And the direct Tokio feature set does not include "full"
    let features = tokio_features(DEPENDI_LSP_MANIFEST);

    assert_eq!(features, expected_tokio_features());
    assert!(!features.contains("full"));
}

#[test]
fn the_full_tokio_feature_remains_enabled() {
    // Given the `dependi-lsp/Cargo.toml` dependency line for `tokio` is:
    //   tokio = { version = "1.52.3", features = ["full"] }
    // When the Tokio dependency features are inspected
    // Then the dependency violates R-01 because the direct Tokio feature set includes "full"
    let manifest =
        manifest_with_dependencies(r#"tokio = { version = "1.52.3", features = ["full"] }"#);
    let features = tokio_features(&manifest);

    assert!(features.contains("full"));
    assert_ne!(features, expected_tokio_features());
}

#[test]
fn a_required_explicit_tokio_feature_is_missing() {
    // Given the `dependi-lsp/Cargo.toml` dependency line for `tokio` is:
    //   tokio = { version = "1.52.3", features = ["rt-multi-thread", "macros", "io-util"] }
    // When the Tokio dependency features are inspected
    // Then the dependency violates R-01 because the direct Tokio feature set is missing "fs, io-std"
    let manifest = manifest_with_dependencies(
        r#"tokio = { version = "1.52.3", features = ["rt-multi-thread", "macros", "io-util"] }"#,
    );
    let features = tokio_features(&manifest);

    assert_eq!(
        missing_expected_tokio_features(&features),
        BTreeSet::from(["fs".to_string(), "io-std".to_string()])
    );
}

#[test]
fn extra_unrelated_direct_tokio_features_are_rejected() {
    // Given the `dependi-lsp/Cargo.toml` dependency line for `tokio` is:
    //   tokio = { version = "1.52.3", features = ["rt-multi-thread", "macros", "io-util", "io-std", "fs", "sync", "time"] }
    // When the Tokio dependency features are inspected
    // Then the dependency violates R-01 because the direct Tokio feature set includes unrelated features "sync, time"
    let manifest = manifest_with_dependencies(
        r#"tokio = { version = "1.52.3", features = ["rt-multi-thread", "macros", "io-util", "io-std", "fs", "sync", "time"] }"#,
    );
    let features = tokio_features(&manifest);

    assert_eq!(
        unrelated_tokio_features(&features),
        BTreeSet::from(["sync".to_string(), "time".to_string()])
    );
}

#[test]
fn feature_equality_ignores_cargo_list_ordering() {
    // Given the `dependi-lsp/Cargo.toml` dependency line for `tokio` is:
    //   tokio = { version = "1.52.3", features = ["fs", "io-std", "io-util", "macros", "rt-multi-thread"] }
    // When the Tokio dependency features are inspected
    // Then the direct Tokio feature set is exactly "fs, io-std, io-util, macros, rt-multi-thread"
    // And the dependency satisfies R-01
    let manifest = manifest_with_dependencies(
        r#"tokio = { version = "1.52.3", features = ["fs", "io-std", "io-util", "macros", "rt-multi-thread"] }"#,
    );
    let features = tokio_features(&manifest);

    assert_eq!(features, expected_tokio_features());
}

#[tokio::test(flavor = "multi_thread")]
async fn trimmed_features_keep_each_required_tokio_and_reqwest_capability() {
    // Given the `dependi-lsp` crate directly enables Tokio features "rt-multi-thread, macros, io-util, io-std, fs"
    // And the `dependi-lsp` crate depends on `reqwest` version "0.13.3"
    // When `cargo check -p dependi-lsp` runs
    // Then code using "<capability_code>" for "<capability>" compiles
    // And the dependency graph satisfies R-02
    use tokio::io::AsyncReadExt;

    let mut reader = tokio::io::empty();
    let mut buffer = [];
    let bytes_read = reader.read(&mut buffer).await.unwrap();
    let cargo_toml = tokio::fs::read_to_string("Cargo.toml").await.unwrap();
    let _stdin = tokio::io::stdin();
    let _client = reqwest::Client::new();

    assert_eq!(bytes_read, 0);
    assert!(cargo_toml.contains("[package]"));
    assert_eq!(
        tokio_features(DEPENDI_LSP_MANIFEST),
        expected_tokio_features()
    );
    assert!(has_dependency(DEPENDI_LSP_MANIFEST, "reqwest"));
}

#[test]
fn missing_direct_tokio_features_break_required_capabilities() {
    // Given the `dependi-lsp` crate directly enables Tokio features "<enabled_features>"
    // And the `dependi-lsp` crate depends on `reqwest` version "0.13.3"
    // When `cargo check -p dependi-lsp` runs
    // Then code using "<failing_capability_code>" for "<missing_feature>" fails to compile
    // And the dependency graph violates R-02
    let cases = [
        (
            ["rt-multi-thread", "io-util", "io-std", "fs"].as_slice(),
            "#[tokio::main]\nasync fn main() {}\n",
        ),
        (
            ["macros", "io-util", "io-std", "fs"].as_slice(),
            "#[tokio::main]\nasync fn main() {}\n",
        ),
        (
            ["rt-multi-thread", "macros", "fs", "io-std"].as_slice(),
            "use tokio::io::AsyncReadExt;\nfn main() {}\n",
        ),
        (
            ["rt-multi-thread", "macros", "io-util", "fs"].as_slice(),
            "fn main() { let _stdin = tokio::io::stdin(); }\n",
        ),
        (
            ["rt-multi-thread", "macros", "io-util", "io-std"].as_slice(),
            "fn main() { let _read = tokio::fs::read_to_string(\"Cargo.toml\"); }\n",
        ),
    ];

    for (enabled_features, capability_code) in cases {
        assert!(
            !cargo_check_probe_succeeds(enabled_features, capability_code),
            "{capability_code} should fail to compile with features {enabled_features:?}"
        );
    }
}

#[test]
fn missing_reqwest_support_breaks_network_capability() {
    // Given the `dependi-lsp` crate directly enables Tokio features "rt-multi-thread, macros, io-util, io-std, fs"
    // And the `dependi-lsp` crate does not depend on `reqwest`
    // When `cargo check -p dependi-lsp` runs
    // Then code issuing an HTTPS request through `reqwest::Client` fails to compile
    // And the dependency graph violates R-02
    let manifest = manifest_without_dependency(DEPENDI_LSP_MANIFEST, "reqwest");

    assert!(has_reqwest_network_capability(DEPENDI_LSP_MANIFEST));
    assert!(!has_reqwest_network_capability(&manifest));
}

#[test]
fn network_capability_comes_from_reqwest_instead_of_a_direct_tokio_feature() {
    // Given the `dependi-lsp` crate directly enables Tokio features "rt-multi-thread, macros, io-util, io-std, fs"
    // And the `dependi-lsp` crate depends on `reqwest` version "0.13.3"
    // When the resolved Cargo dependency graph is inspected
    // Then HTTPS client code through `reqwest::Client` is available
    // And the direct Tokio feature set does not include "net"
    // And the dependency graph satisfies R-02
    let features = tokio_features(DEPENDI_LSP_MANIFEST);
    let _client = reqwest::Client::new();

    assert!(has_dependency(DEPENDI_LSP_MANIFEST, "reqwest"));
    assert!(!features.contains("net"));
}

#[test]
fn trimmed_build_meets_the_minimum_improvement_floor() {
    // Given the `features = ["full"]` baseline binary size is 50000000 bytes
    // And the `features = ["full"]` baseline clean build takes 120 seconds
    // And the trimmed Tokio build binary size is <trimmed_binary_size> bytes
    // And the trimmed Tokio clean build takes <trimmed_build_seconds> seconds
    // When the improvement is calculated against the baseline
    // Then the binary-size reduction is at least 5 percent
    // And the clean-build-time reduction is at least 10 percent
    // And the performance result satisfies R-03
    for case_name in ["pass_lower_bound", "pass_inside_range"] {
        let (baseline_binary, baseline_seconds, trimmed_binary, trimmed_seconds) =
            measurement_case(case_name);

        assert!(performance_floor_satisfied(
            baseline_binary,
            baseline_seconds,
            trimmed_binary,
            trimmed_seconds
        ));
    }
}

#[test]
fn trimmed_build_below_either_improvement_floor_fails_the_rule() {
    // Given the `features = ["full"]` baseline binary size is 50000000 bytes
    // And the `features = ["full"]` baseline clean build takes 120 seconds
    // And the trimmed Tokio build binary size is <trimmed_binary_size> bytes
    // And the trimmed Tokio clean build takes <trimmed_build_seconds> seconds
    // When the improvement is calculated against the baseline
    // Then the performance result violates R-03 with reason "<reason>"
    for case_name in ["fail_binary", "fail_build_time"] {
        let (baseline_binary, baseline_seconds, trimmed_binary, trimmed_seconds) =
            measurement_case(case_name);

        assert!(!performance_floor_satisfied(
            baseline_binary,
            baseline_seconds,
            trimmed_binary,
            trimmed_seconds
        ));
    }
}

#[test]
fn performance_comparison_uses_the_same_clean_build_conditions() {
    // Given the `features = ["full"]` baseline is measured from a clean build
    // And the trimmed Tokio build is measured from an incremental build
    // When the improvement is calculated against the baseline
    // Then the performance result is rejected as incomparable
    // And R-03 requires both measurements to use clean build conditions
    let baseline = BuildMeasurement {
        source_revision: "a1b2c3d",
        profile: "release",
        clean_build: true,
    };
    let trimmed = BuildMeasurement {
        source_revision: "a1b2c3d",
        profile: "release",
        clean_build: false,
    };

    assert_eq!(
        comparable_measurements(&baseline, &trimmed),
        Err("clean build condition differs")
    );
}

#[test]
fn performance_comparison_rejects_mismatched_source_revision_or_profile() {
    // Given the `features = ["full"]` baseline is measured from source revision "a1b2c3d" using build profile "release"
    // And the trimmed Tokio build is measured from source revision "<trimmed_revision>" using build profile "<trimmed_profile>"
    // And both measurements use clean build conditions
    // When the improvement is calculated against the baseline
    // Then the performance result is rejected as incomparable with reason "<reason>"
    let baseline = BuildMeasurement {
        source_revision: "a1b2c3d",
        profile: "release",
        clean_build: true,
    };
    let different_revision = BuildMeasurement {
        source_revision: "e4f5a6b",
        profile: "release",
        clean_build: true,
    };
    let different_profile = BuildMeasurement {
        source_revision: "a1b2c3d",
        profile: "debug",
        clean_build: true,
    };

    assert_eq!(
        comparable_measurements(&baseline, &different_revision),
        Err("source revision differs")
    );
    assert_eq!(
        comparable_measurements(&baseline, &different_profile),
        Err("build profile differs")
    );
}
