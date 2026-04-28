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

use std::fs;
use std::path::PathBuf;

use hashbrown::HashSet;
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
    let content =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing docs/architecture.md: {e}"));
    (path, content)
}

/// Collect names of `pub struct|enum|trait` items reachable from outside the
/// crate's `tests` and private-module scopes.
///
/// Scope and limits — this is a textual scan, not a true public-API resolver.
/// It deliberately accepts:
///   - `pub`, `pub(crate)`, and `pub(in …)` items at any nesting (no visibility
///     parsing — `pub` in source is treated as public).
///
/// It deliberately filters out:
///   - items inside `#[cfg(test)]` modules (cfg-gated test-only code), and
///   - items inside non-`pub mod NAME { … }` blocks (items not reachable
///     outside the file from a top-level entry point).
///
/// It does NOT:
///   - resolve `pub use` re-exports (a name re-exported from a private
///     submodule still gets picked up because the original definition uses
///     `pub`),
///   - parse the AST (a `syn`-based resolver would be more rigorous; the cost
///     is a heavier dev dependency for a doc-validator).
///
/// The brace-depth tracking can desync on braces inside string literals or
/// comments. The failure mode is a false-negative (an item is excluded that
/// wouldn't have been), which is acceptable for a validator whose job is to
/// flag *missing* claims rather than enforce a closed set.
fn collect_pub_defs(root: &std::path::Path) -> HashSet<String> {
    let item_re = Regex::new(r"^\s*pub(?:\([^)]*\))?\s+(?:struct|enum|trait)\s+(\w+)").unwrap();
    let cfg_test_re = Regex::new(r"^\s*#\[cfg\(test\)\]").unwrap();
    // Match `mod NAME {` without a leading `pub` (i.e. private modules).
    let priv_mod_re = Regex::new(r"^\s*(?:pub\([^)]*\)\s+)?mod\s+\w+\s*\{").unwrap();
    let pub_mod_re = Regex::new(r"^\s*pub(?:\([^)]*\))?\s+mod\b").unwrap();

    let mut set = HashSet::new();
    for entry in WalkDir::new(root).into_iter().flatten() {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let Ok(src) = fs::read_to_string(entry.path()) else {
            continue;
        };

        let mut pending_cfg_test = false;
        let mut skip_depth: i32 = 0;
        for line in src.lines() {
            if skip_depth > 0 {
                skip_depth += line.matches('{').count() as i32;
                skip_depth -= line.matches('}').count() as i32;
                if skip_depth <= 0 {
                    skip_depth = 0;
                }
                continue;
            }
            if pending_cfg_test {
                if line.contains('{') {
                    skip_depth =
                        line.matches('{').count() as i32 - line.matches('}').count() as i32;
                    if skip_depth <= 0 {
                        skip_depth = 0;
                        pending_cfg_test = false;
                    }
                    continue;
                }
                pending_cfg_test = false;
            }
            if cfg_test_re.is_match(line) {
                pending_cfg_test = true;
                continue;
            }
            if !pub_mod_re.is_match(line) && priv_mod_re.is_match(line) {
                skip_depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if skip_depth <= 0 {
                    skip_depth = 0;
                }
                continue;
            }
            if let Some(c) = item_re.captures(line) {
                set.insert(c[1].to_string());
            }
        }
    }
    set
}

#[test]
fn architecture_doc_exists_and_nonempty() {
    let (path, content) = read_doc();
    let required_sections = [
        "## 1. Introduction",
        "## 2. Top-Level Architecture",
        "## 3. Request Lifecycle",
        "## 4. Core Data Structures",
        "## 7. Cache Strategy",
        "## 11. Key Design Decisions",
    ];
    for section in &required_sections {
        let display = path.display();
        assert!(
            content.contains(section),
            "{display} is missing required section header `{section}`"
        );
    }
    let display = path.display();
    assert!(
        content.contains("```mermaid"),
        "{display} contains no ```mermaid fenced block — architecture diagrams are required"
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
fn architecture_doc_has_four_mermaid_diagrams() {
    let (path, content) = read_doc();
    let count = content.matches("```mermaid").count();
    let path_disp = path.display();
    assert!(
        count >= 4,
        "{path_disp} has only {count} mermaid block(s); expected >= 4"
    );
}

#[test]
fn architecture_doc_module_paths_exist() {
    let root = workspace_root();
    let (_, content) = read_doc();
    let re = Regex::new(r"dependi-lsp/src/[\w/]+\.rs(?::(\d+))?").unwrap();

    let mut errors = Vec::new();
    let mut cite_count = 0usize;
    for cap in re.captures_iter(&content) {
        cite_count += 1;
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
        cite_count > 0,
        "no `dependi-lsp/src/...rs` citations found in docs/architecture.md — \
         the architecture guide must cite source files for the validator to be meaningful"
    );
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

    let claim_re = Regex::new(r"`([A-Z][A-Za-z0-9_]+)`\s+(?:struct|trait|enum)\b").unwrap();
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
