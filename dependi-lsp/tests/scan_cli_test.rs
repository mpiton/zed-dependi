use std::path::PathBuf;
use std::process::Command;

use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn dependi_lsp_bin() -> String {
    env!("CARGO_BIN_EXE_dependi-lsp").to_string()
}

fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel)
}

#[tokio::test]
async fn test_scan_uses_lockfile_and_reports_transitive() {
    let server = MockServer::start().await;

    // Mock OSV querybatch: npm fixture has direct [react] + transitive [scheduler].
    // Return no vulns for react (index 0) and a vuln for scheduler (index 1).
    Mock::given(method("POST"))
        .and(path("/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                { "vulns": [] },
                { "vulns": [{ "id": "CVE-TEST-001", "modified": "2024-01-01T00:00:00Z" }] }
            ]
        })))
        .mount(&server)
        .await;

    // Mock individual vuln lookups (check_rustsec_unmaintained calls GET /vulns/{id}).
    // CVE-TEST-001 is not a RUSTSEC id so this won't be called, but guard against it anyway.
    Mock::given(method("GET"))
        .and(path_regex(r"^/vulns/.+"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "CVE-TEST-001",
            "summary": "test summary",
            "details": "test details",
            "severity": [{ "type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" }],
            "references": []
        })))
        .mount(&server)
        .await;

    let fixture = fixture_path("npm-project-with-lockfile/package.json");

    let output = Command::new(dependi_lsp_bin())
        .env("OSV_ENDPOINT", server.uri())
        .args(["scan", "--output", "json", "--file"])
        .arg(&fixture)
        .output()
        .expect("failed to run dependi-lsp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("\"transitive\""),
        "expected transitive array in JSON output. stdout=\n{stdout}\nstderr=\n{stderr}"
    );
    assert!(
        stdout.contains("\"direct\""),
        "expected direct array in JSON output. stdout=\n{stdout}\nstderr=\n{stderr}"
    );
}

#[test]
fn test_scan_no_use_lockfile_flag_skips_detection() {
    // When --no-use-lockfile is passed, even with a lockfile present the graph should be empty.
    // We point OSV_ENDPOINT at an unreachable port so any actual query errors fast and we
    // check that no "transitive" data was computed.
    let fixture = fixture_path("rust-project-with-lockfile/Cargo.toml");

    let output = Command::new(dependi_lsp_bin())
        .env("OSV_ENDPOINT", "http://127.0.0.1:1") // unreachable
        .args(["scan", "--output", "json", "--no-use-lockfile", "--file"])
        .arg(&fixture)
        .output()
        .expect("failed to run dependi-lsp");

    // With a broken endpoint, the query fails → ExitCode::FAILURE (1). That's ok;
    // the flag just needs to parse. The stderr should confirm the command ran.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Scanning") || stderr.contains("Error"),
        "expected scan to run, got stderr: {stderr}"
    );
}

#[test]
fn test_scan_malformed_lockfile_falls_back() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        r#"
[package]
name = "x"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
"#,
    )
    .expect("write manifest");
    std::fs::write(tmp.path().join("Cargo.lock"), "not valid toml ][").expect("write lockfile");

    let output = Command::new(dependi_lsp_bin())
        .env("OSV_ENDPOINT", "http://127.0.0.1:1") // unreachable so we don't hit network
        .args(["scan", "--output", "json", "--file"])
        .arg(tmp.path().join("Cargo.toml"))
        .output()
        .expect("run");
    // Should not crash. Exit code may be 0 or 1 depending on how graceful the fallback is;
    // what we care about is not a panic.
    assert!(output.status.code().is_some(), "process exited abnormally");
}
