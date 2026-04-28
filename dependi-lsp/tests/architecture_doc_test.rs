//! TDD guard for `docs/architecture.md` (issue #232).
//!
//! Hard fails:
//!   - cited `dependi-lsp/src/...rs` files exist
//!   - if line suffix `:N`, file has >= N lines
//!   - inline-code claims like `Foo` struct / `Foo` trait / `Foo` enum
//!     match a real `pub struct|enum|trait` definition in `dependi-lsp/src/`
//!
//! Soft warn (stderr):
//!   - PascalCase identifiers in backticks not in allowlist or source

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use regex::Regex;
use walkdir::WalkDir;

const PASCAL_ALLOWLIST: &[&str] = &[
    "Arc",
    "RwLock",
    "Mutex",
    "DashMap",
    "String",
    "Vec",
    "Url",
    "HashMap",
    "Instant",
    "Duration",
    "JoinHandle",
    "Semaphore",
    "Result",
    "Option",
    "Client",
    "HeaderMap",
    "Diagnostic",
    "InlayHint",
    "Range",
    "Position",
    "CodeActionOrCommand",
    "CompletionItem",
    "DocumentLink",
    "Path",
    "PathBuf",
    "AtomicU64",
    "DateTime",
    "Utc",
    "TOC",
    "OSV",
    "RUSTSEC",
    "LSP",
    "API",
    "CSV",
    "JSON",
    "TOML",
    "YAML",
    "HTTP",
    "HTTPS",
    "CVE",
    "CVSS",
    "TODO",
    "TBD",
    "WAL",
    "PRAGMA",
    "Tokio",
    "Rust",
    "Zed",
    "GitHub",
    "Jekyll",
    "MIT",
    "Cargo",
    "Some",
    "None",
    "Err",
    "Ok",
    "DAG",
    "DNS",
    "TCP",
    "TLS",
    "UTF",
    "JSON-RPC",
    "WASM",
    "DFS",
    "ID",
    "URL",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has parent")
        .to_path_buf()
}

fn read_doc() -> (PathBuf, String) {
    let path = workspace_root().join("docs/architecture.md");
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing docs/architecture.md: {e}"));
    (path, content)
}

fn collect_pub_defs(root: &std::path::Path) -> HashSet<String> {
    let re = Regex::new(r"^\s*pub (?:struct|enum|trait) (\w+)").unwrap();
    let mut set = HashSet::new();
    for entry in WalkDir::new(root).into_iter().flatten() {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let Ok(src) = fs::read_to_string(entry.path()) else {
            continue;
        };
        for line in src.lines() {
            if let Some(c) = re.captures(line) {
                set.insert(c[1].to_string());
            }
        }
    }
    set
}

#[test]
fn architecture_doc_exists_and_nonempty() {
    let (path, content) = read_doc();
    assert!(
        content.len() > 5000,
        "{} is only {} bytes — needs real content",
        path.display(),
        content.len()
    );
}

#[test]
fn architecture_doc_has_no_placeholder_comments() {
    let (path, content) = read_doc();
    let placeholder_re = Regex::new(r"<!--\s*to be filled").unwrap();
    let hits: Vec<&str> = placeholder_re
        .find_iter(&content)
        .map(|m| m.as_str())
        .collect();
    assert!(
        hits.is_empty(),
        "{} still contains {} placeholder comment(s) — sections not yet written",
        path.display(),
        hits.len()
    );
}

#[test]
fn architecture_doc_has_three_mermaid_diagrams() {
    let (path, content) = read_doc();
    let count = content.matches("```mermaid").count();
    assert!(
        count >= 3,
        "{} has only {} mermaid block(s); expected at least 3 (system boundary, lifecycle, cache, vuln)",
        path.display(),
        count
    );
}

#[test]
fn architecture_doc_module_paths_exist() {
    let root = workspace_root();
    let (_, content) = read_doc();
    let re = Regex::new(r"dependi-lsp/src/[\w/]+\.rs(?::(\d+))?").unwrap();

    let mut errors = Vec::new();
    for cap in re.captures_iter(&content) {
        let full = cap.get(0).unwrap().as_str();
        let (path_part, line_opt) = match cap.get(1) {
            Some(line_match) => {
                let path_only = &full[..full.len() - line_match.as_str().len() - 1];
                (path_only, line_match.as_str().parse::<usize>().ok())
            }
            None => (full, None),
        };
        let abs = root.join(path_part);
        if !abs.exists() {
            errors.push(format!("missing file: {path_part}"));
            continue;
        }
        if let Some(line) = line_opt {
            let n = fs::read_to_string(&abs)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if n < line {
                errors.push(format!("{path_part}:{line} but file has {n} lines"));
            }
        }
    }
    assert!(
        errors.is_empty(),
        "broken cites:\n  {}",
        errors.join("\n  ")
    );
}

#[test]
fn architecture_doc_struct_trait_enum_claims_exist_in_source() {
    let root = workspace_root();
    let (_, content) = read_doc();
    let allowed = collect_pub_defs(&root.join("dependi-lsp/src"));

    let claim_re =
        Regex::new(r"`([A-Z][A-Za-z0-9_]+)`\s+(?:struct|trait|enum)\b").unwrap();
    let mut bad = Vec::new();
    for cap in claim_re.captures_iter(&content) {
        let name = cap[1].to_string();
        if !allowed.contains(&name) {
            bad.push(name);
        }
    }
    bad.sort();
    bad.dedup();
    assert!(
        bad.is_empty(),
        "doc claims these are struct/trait/enum but no `pub` def found: {bad:?}"
    );
}

#[test]
fn architecture_doc_pascal_case_soft_warn() {
    let root = workspace_root();
    let (_, content) = read_doc();
    let mut allowed = collect_pub_defs(&root.join("dependi-lsp/src"));
    for name in PASCAL_ALLOWLIST {
        allowed.insert((*name).to_string());
    }

    let re = Regex::new(r"`([A-Z][A-Za-z0-9_]+)`").unwrap();
    let mut unknown = Vec::new();
    for cap in re.captures_iter(&content) {
        let name = cap[1].to_string();
        if !allowed.contains(&name) {
            unknown.push(name);
        }
    }
    unknown.sort();
    unknown.dedup();
    if !unknown.is_empty() {
        eprintln!(
            "WARN architecture_doc PascalCase names not in source or allowlist (soft check): {unknown:?}"
        );
    }
}
