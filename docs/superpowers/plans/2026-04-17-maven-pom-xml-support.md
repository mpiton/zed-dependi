# Maven / pom.xml Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add full support for Java/Maven (`pom.xml`) as a 9th ecosystem in Dependi LSP, with parser, Maven Central registry client, OSV vulnerability scanning, and parity with the 8 existing ecosystems.

**Architecture:** Implements the existing `Parser` and `Registry` traits. New `MavenParser` uses `quick-xml` streaming for a 2-pass parse (properties, then dependencies). New `MavenCentralRegistry` fetches `maven-metadata.xml` for versions plus a best-effort POM fetch for license/description. Wiring goes through `file_types.rs`, `backend.rs`, and `vulnerabilities/mod.rs` with identical patterns to existing ecosystems.

**Tech Stack:** Rust 2024, `quick-xml` 0.38, `reqwest`, `anyhow`, `tokio`. Test patterns: inline `#[cfg(test)]` modules (no mocking framework).

**Spec reference:** `docs/superpowers/specs/2026-04-17-maven-pom-xml-support-design.md`

---

## File Structure

**New files:**
- `dependi-lsp/src/parsers/maven.rs` — `MavenParser` (Parser trait impl + helpers)
- `dependi-lsp/src/registries/maven_central.rs` — `MavenCentralRegistry` (Registry trait impl + XML parsing helpers + version comparator)

**Modified files:**
- `dependi-lsp/Cargo.toml` — add `quick-xml` dependency
- `dependi-lsp/src/parsers/mod.rs` — add `pub mod maven;`
- `dependi-lsp/src/registries/mod.rs` — add `pub mod maven_central;` + doc table
- `dependi-lsp/src/file_types.rs` — enum variant, detect, ecosystem, url, name, cache_key
- `dependi-lsp/src/vulnerabilities/mod.rs` — enum variant + as_osv_str
- `dependi-lsp/src/config.rs` — `MavenRegistryConfig` + field in `RegistriesConfig`
- `dependi-lsp/src/backend.rs` — imports, struct fields, parse_document, get_version_info, fetch loop, init, context clone
- `CHANGELOG.md` — `[Unreleased]` entry
- `dependi-lsp/tests/integration_test.rs` — end-to-end flow test (optional)

**NOT modified (intentionally):**
- `providers/code_actions.rs` — already has fallback `_ => VersionUpdateType::Patch` on line 56 (handles SNAPSHOT versions)
- `providers/inlay_hints.rs`, `providers/diagnostics.rs`, `providers/document_links.rs`, `providers/completion.rs` — agnostic to FileType
- `reports.rs` — agnostic

---

## Task 1: Add `quick-xml` dependency

**Files:**
- Modify: `dependi-lsp/Cargo.toml:38` (add after line 38, inside `[dependencies]`)

- [ ] **Step 1: Add quick-xml to Cargo.toml**

Insert this line in the `[dependencies]` block of `dependi-lsp/Cargo.toml`, after the `hashbrown` line:

```toml
quick-xml = "0.38"
```

- [ ] **Step 2: Verify dependency resolves**

Run: `cd dependi-lsp && cargo check --lib`
Expected: Compiles successfully (or at most warnings). Quick-xml should download and be cached.

- [ ] **Step 3: Commit**

```bash
git add dependi-lsp/Cargo.toml dependi-lsp/Cargo.lock
git commit -m "deps(dependi-lsp): add quick-xml 0.38 for Maven pom.xml parsing"
```

---

## Task 2: Add `Ecosystem::Maven` variant

**Files:**
- Modify: `dependi-lsp/src/vulnerabilities/mod.rs:27` (add variant before closing brace of enum)
- Modify: `dependi-lsp/src/vulnerabilities/mod.rs:42` (add match arm before closing brace)

- [ ] **Step 1: Write failing test**

Add this test inside `#[cfg(test)] mod tests` in `dependi-lsp/src/vulnerabilities/mod.rs` (after the existing tests, before the closing `}`):

```rust
    #[test]
    fn test_ecosystem_maven_as_osv_str() {
        use super::Ecosystem;
        assert_eq!(Ecosystem::Maven.as_osv_str(), "Maven");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd dependi-lsp && cargo test --lib vulnerabilities::tests::test_ecosystem_maven_as_osv_str`
Expected: FAIL with "no variant or associated item named `Maven`".

- [ ] **Step 3: Add `Maven` variant and match arm**

In `dependi-lsp/src/vulnerabilities/mod.rs`:

Modify the enum (currently ending at line 28) to add a `Maven` variant before the closing brace:

```rust
pub enum Ecosystem {
    /// Rust crates (crates.io)
    CratesIo,
    /// JavaScript/Node packages (npm)
    Npm,
    /// Python packages (PyPI)
    PyPI,
    /// Go modules
    Go,
    /// PHP packages (Packagist)
    Packagist,
    /// Dart/Flutter packages (pub.dev)
    Pub,
    /// .NET packages (NuGet)
    NuGet,
    /// Ruby gems (RubyGems.org)
    RubyGems,
    /// Java packages (Maven Central)
    Maven,
}
```

Modify `as_osv_str()` (currently ending at line 43) to add the match arm:

```rust
    pub fn as_osv_str(&self) -> &'static str {
        match self {
            Ecosystem::CratesIo => "crates.io",
            Ecosystem::Npm => "npm",
            Ecosystem::PyPI => "PyPI",
            Ecosystem::Go => "Go",
            Ecosystem::Packagist => "Packagist",
            Ecosystem::Pub => "Pub",
            Ecosystem::NuGet => "NuGet",
            Ecosystem::RubyGems => "RubyGems",
            Ecosystem::Maven => "Maven",
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd dependi-lsp && cargo test --lib vulnerabilities::tests::test_ecosystem_maven_as_osv_str`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add dependi-lsp/src/vulnerabilities/mod.rs
git commit -m "feat(vulnerabilities): add Ecosystem::Maven variant for OSV.dev"
```

---

## Task 3: Add `FileType::Maven` variant + detection

**Files:**
- Modify: `dependi-lsp/src/file_types.rs:33` (add variant before closing brace of enum)
- Modify: `dependi-lsp/src/file_types.rs:61` (add detect branch before the `hatch.toml` branch)

- [ ] **Step 1: Write failing test**

Add these tests inside `#[cfg(test)] mod tests` in `dependi-lsp/src/file_types.rs` (after `test_detect_ruby`):

```rust
    #[test]
    fn test_detect_maven() {
        let uri = Url::parse("file:///project/pom.xml").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Maven));

        let uri = Url::parse("file:///project/subdir/pom.xml").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Maven));

        // Must not match similar-looking names
        let uri = Url::parse("file:///project/mypom.xml").unwrap();
        assert_eq!(FileType::detect(&uri), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd dependi-lsp && cargo test --lib file_types::tests::test_detect_maven`
Expected: FAIL with "no variant or associated item named `Maven`".

- [ ] **Step 3: Add `Maven` variant to FileType enum**

In `dependi-lsp/src/file_types.rs`, modify the enum (around lines 17-34) to add a `Maven` variant:

```rust
pub enum FileType {
    /// Rust packages (Cargo.toml)
    Cargo,
    /// JavaScript/Node.js packages (package.json)
    Npm,
    /// Python packages (requirements.txt, constraints.txt, pyproject.toml)
    Python,
    /// Go modules (go.mod)
    Go,
    /// PHP packages (composer.json)
    Php,
    /// Dart/Flutter packages (pubspec.yaml)
    Dart,
    /// C#/.NET packages (*.csproj)
    Csharp,
    /// Ruby gems (Gemfile)
    Ruby,
    /// Java packages (Maven, pom.xml)
    Maven,
}
```

- [ ] **Step 4: Add detection branch**

In `dependi-lsp/src/file_types.rs`, modify `detect()` to add a branch for `pom.xml`. Insert this branch **BEFORE** the `hatch.toml` branch (currently at line 63). The updated section should look like:

```rust
        } else if path.ends_with("Gemfile") {
            Some(FileType::Ruby)
        } else if filename == "pom.xml" {
            // Maven project object model; the filename is always `pom.xml`.
            Some(FileType::Maven)
        } else if filename == "hatch.toml" {
```

Using exact-match `filename == "pom.xml"` (not `path.ends_with`) mirrors `hatch.toml` handling to avoid false positives on files like `not-pom.xml` or `mypom.xml`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cd dependi-lsp && cargo test --lib file_types::tests::test_detect_maven`
Expected: PASS.

Also verify the match in `parse_document` / `process_document` now triggers an `unreachable` / non-exhaustive warning because Maven has no branch yet. This will be resolved in Task 8. For now ensure the library still compiles:

Run: `cd dependi-lsp && cargo build --lib 2>&1 | head -30`
Expected: Build fails with non-exhaustive `match` errors in `backend.rs` — those branches will be added in Task 8. Stop at this task for review if anything else is reported.

Note: The match arms in `backend.rs::parse_document` and `ProcessingContext::parse_document` and `get_version_info` do not use a wildcard `_` — they enumerate all variants explicitly, so adding `Maven` triggers compile errors. This is INTENTIONAL and will be addressed in Task 8.

Rather than committing a broken build, we continue directly to Task 4 which is additive within `file_types.rs` only.

**Do NOT commit yet — continue to Task 4.**

---

## Task 4: Add Maven mappings in file_types.rs

**Files:**
- Modify: `dependi-lsp/src/file_types.rs:85` (add match arm in `to_ecosystem`)
- Modify: `dependi-lsp/src/file_types.rs:101` (add match arm in `fmt_registry_package_url`)
- Modify: `dependi-lsp/src/file_types.rs:115` (add match arm in `registry_name`)
- Modify: `dependi-lsp/src/file_types.rs:144` (add match arm in `fmt_cache_key`)

- [ ] **Step 1: Write failing tests**

Add these tests inside `#[cfg(test)] mod tests` in `dependi-lsp/src/file_types.rs`, after `test_detect_maven`:

```rust
    #[test]
    fn test_to_ecosystem_maven() {
        assert_eq!(FileType::Maven.to_ecosystem(), Ecosystem::Maven);
    }

    #[test]
    fn test_cache_key_maven() {
        assert_eq!(
            FileType::Maven.cache_key("org.slf4j:slf4j-api"),
            "maven:org.slf4j:slf4j-api"
        );
    }

    #[test]
    fn test_registry_package_url_maven() {
        // groupId:artifactId → groupId/artifactId in the URL path
        assert_eq!(
            FileType::Maven.registry_package_url("org.slf4j:slf4j-api"),
            "https://mvnrepository.com/artifact/org.slf4j/slf4j-api"
        );
    }

    #[test]
    fn test_registry_name_maven() {
        assert_eq!(FileType::Maven.registry_name(), "Maven Central");
    }
```

- [ ] **Step 2: Add match arms**

In `dependi-lsp/src/file_types.rs`:

Modify `to_ecosystem()` — add arm after `Ruby` (currently around line 84):

```rust
            FileType::Ruby => Ecosystem::RubyGems,
            FileType::Maven => Ecosystem::Maven,
```

Modify `fmt_registry_package_url()` — add arm after `Csharp` (currently around line 100). Note the colon-to-slash transformation for Maven coordinates:

```rust
            FileType::Csharp => write!(f, "https://www.nuget.org/packages/{name}"),
            FileType::Maven => {
                // Maven coordinate "groupId:artifactId" → URL path "groupId/artifactId"
                let url_path = name.replace(':', "/");
                write!(f, "https://mvnrepository.com/artifact/{url_path}")
            }
```

Modify `registry_name()` — add arm after `Csharp` (currently around line 114):

```rust
            FileType::Csharp => "NuGet",
            FileType::Maven => "Maven Central",
```

Modify `fmt_cache_key()` — add arm after `Ruby` (currently around line 143):

```rust
            FileType::Ruby => write!(f, "rubygems:{package_name}"),
            FileType::Maven => write!(f, "maven:{package_name}"),
```

- [ ] **Step 3: Run tests to verify they pass**

Run:
```bash
cd dependi-lsp && cargo test --lib file_types::tests::test_to_ecosystem_maven \
  file_types::tests::test_cache_key_maven \
  file_types::tests::test_registry_package_url_maven \
  file_types::tests::test_registry_name_maven \
  file_types::tests::test_detect_maven
```
Expected: All 5 tests PASS.

Note: Backend still does not compile because `parse_document`, `get_version_info`, fetch loop match are still non-exhaustive. This is expected — fixed in Task 8.

- [ ] **Step 4: Commit (file_types + vulnerabilities)**

Commit the Maven enum plumbing together since it's all ecosystem-level metadata:

```bash
git add dependi-lsp/src/file_types.rs
git commit -m "feat(file_types): add FileType::Maven with detection and mappings"
```

---

## Task 5: Create `MavenRegistryConfig` in config.rs

**Files:**
- Modify: `dependi-lsp/src/config.rs:183` (add struct after `CargoRegistryConfig`, before `RegistriesConfig`)
- Modify: `dependi-lsp/src/config.rs:187-193` (add field in `RegistriesConfig`)

- [ ] **Step 1: Write failing test**

Add this test inside `#[cfg(test)] mod tests` in `dependi-lsp/src/config.rs` (add to existing tests section, near `test_default_config`):

```rust
    #[test]
    fn test_maven_registry_config_default() {
        let config = Config::default();
        assert_eq!(
            config.registries.maven.url,
            "https://repo1.maven.org/maven2"
        );
    }

    #[test]
    fn test_maven_registry_config_custom_url() {
        let json = json!({
            "registries": {
                "maven": {
                    "url": "https://nexus.internal.corp/repository/maven-public"
                }
            }
        });
        let config = Config::from_init_options(Some(json));
        assert_eq!(
            config.registries.maven.url,
            "https://nexus.internal.corp/repository/maven-public"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd dependi-lsp && cargo test --lib config::tests::test_maven_registry_config`
Expected: FAIL — field `maven` doesn't exist.

- [ ] **Step 3: Add `MavenRegistryConfig` struct + field**

In `dependi-lsp/src/config.rs`, add the struct after `CargoRegistryConfig` (around line 183, before `RegistriesConfig`):

```rust
/// Maven registry configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MavenRegistryConfig {
    /// Base URL for the Maven repository (no trailing slash).
    /// Defaults to Maven Central. Configure to point at Nexus/Artifactory mirrors.
    pub url: String,
}

impl Default for MavenRegistryConfig {
    fn default() -> Self {
        Self {
            url: "https://repo1.maven.org/maven2".to_string(),
        }
    }
}
```

Modify `RegistriesConfig` (currently lines 185-193) to add the `maven` field:

```rust
/// Package registries configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RegistriesConfig {
    /// npm registry configuration
    pub npm: NpmRegistryConfig,
    /// Cargo alternative registries configuration
    #[serde(default)]
    pub cargo: CargoRegistryConfig,
    /// Maven registry configuration (Maven Central by default)
    #[serde(default)]
    pub maven: MavenRegistryConfig,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd dependi-lsp && cargo test --lib config::tests::test_maven_registry_config`
Expected: Both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add dependi-lsp/src/config.rs
git commit -m "feat(config): add MavenRegistryConfig with configurable base URL"
```

---

## Task 6: Create `MavenParser` skeleton with simple dependency test

**Files:**
- Create: `dependi-lsp/src/parsers/maven.rs`
- Modify: `dependi-lsp/src/parsers/mod.rs:62` (add `pub mod maven;` before the closing of the list, after `pub mod ruby;`)

- [ ] **Step 1: Register module**

Add at the end of `dependi-lsp/src/parsers/mod.rs` (after line 62 `pub mod ruby;`):

```rust
pub mod maven;
```

- [ ] **Step 2: Create maven.rs with failing test**

Create `dependi-lsp/src/parsers/maven.rs` with the minimum needed for the first test to compile-but-fail:

```rust
//! Maven (pom.xml) parser for Java projects.
//!
//! Parses direct dependencies declared in `pom.xml` files, including
//! `<dependencyManagement>`, with two passes:
//! 1. Collect `<properties>` for variable substitution (`${...}`).
//! 2. Extract dependencies and substitute property references.
//!
//! The dependency `name` uses the Maven convention `groupId:artifactId`
//! (matching OSV.dev and the mvnrepository.com URL scheme).
//!
//! Unsupported in this MVP (detected but not resolved):
//! - Parent POM inheritance
//! - BOM (`<scope>import</scope>`) resolution from remote POMs
//! - Plugin dependencies

use crate::parsers::{Dependency, Parser};

/// Parser for Maven `pom.xml` files.
#[derive(Default)]
pub struct MavenParser;

impl MavenParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for MavenParser {
    fn parse(&self, _content: &str) -> Vec<Dependency> {
        // TDD scaffold — to be implemented in subsequent tasks.
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_dependency() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>app</artifactId>
    <version>1.0.0</version>
    <dependencies>
        <dependency>
            <groupId>org.slf4j</groupId>
            <artifactId>slf4j-api</artifactId>
            <version>1.7.30</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1, "should parse one dependency");
        assert_eq!(deps[0].name, "org.slf4j:slf4j-api");
        assert_eq!(deps[0].version, "1.7.30");
        assert!(!deps[0].dev);
        assert!(!deps[0].optional);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd dependi-lsp && cargo test --lib parsers::maven::tests::test_parse_simple_dependency`
Expected: FAIL — `deps.len()` is 0, expected 1.

- [ ] **Step 4: Implement minimum parser to pass the test**

Replace the `impl Parser for MavenParser` block in `dependi-lsp/src/parsers/maven.rs` with a working streaming implementation. Replace ALSO the `use` line with the expanded imports. The full file should now be:

```rust
//! Maven (pom.xml) parser for Java projects.
//!
//! Parses direct dependencies declared in `pom.xml` files, including
//! `<dependencyManagement>`, with two passes:
//! 1. Collect `<properties>` for variable substitution (`${...}`).
//! 2. Extract dependencies and substitute property references.
//!
//! The dependency `name` uses the Maven convention `groupId:artifactId`
//! (matching OSV.dev and the mvnrepository.com URL scheme).
//!
//! Unsupported in this MVP (detected but not resolved):
//! - Parent POM inheritance
//! - BOM (`<scope>import</scope>`) resolution from remote POMs
//! - Plugin dependencies

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::parsers::{Dependency, Parser};

/// Parser for Maven `pom.xml` files.
#[derive(Default)]
pub struct MavenParser;

impl MavenParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for MavenParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let properties = extract_properties(content);
        extract_dependencies(content, &properties)
    }
}

/// Precomputed byte-offset → (line, column) mapping for a source string.
/// Lines are 0-indexed, columns are character-based within each line.
fn line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Convert a byte offset to (line, column), both 0-indexed.
fn offset_to_position(offsets: &[usize], byte_offset: usize) -> (u32, u32) {
    let line_idx = match offsets.binary_search(&byte_offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let line_start = offsets[line_idx];
    let col = byte_offset.saturating_sub(line_start);
    (line_idx as u32, col as u32)
}

/// Pass 1: collect `<properties>` entries (name → value) from the pom.
fn extract_properties(_content: &str) -> HashMap<String, String> {
    // Placeholder for Task 7. The simple-dependency test doesn't use properties,
    // so an empty map is sufficient for now.
    HashMap::new()
}

/// Pass 2: extract dependencies from `<dependencies>` and
/// `<dependencyManagement><dependencies>`, substituting `${property}` placeholders.
fn extract_dependencies(
    content: &str,
    properties: &HashMap<String, String>,
) -> Vec<Dependency> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let offsets = line_offsets(content);
    let mut out = Vec::new();

    // State: track which element we're inside.
    let mut in_dependencies = false;
    let mut in_dep_mgmt = false;
    let mut in_plugins = false;
    let mut in_dependency = false;
    let mut current_tag: Option<Vec<u8>> = None;

    // Current dependency accumulator
    let mut cur_group: Option<String> = None;
    let mut cur_artifact: Option<String> = None;
    let mut cur_version: Option<String> = None;
    let mut cur_version_span: Option<(usize, usize)> = None; // byte offsets into content
    let mut cur_scope: Option<String> = None;
    let mut cur_optional = false;

    loop {
        match reader.read_event() {
            Err(_) => return vec![], // invalid XML → empty result
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"dependencies" => in_dependencies = true,
                    b"dependencyManagement" => in_dep_mgmt = true,
                    b"plugins" | b"pluginManagement" => in_plugins = true,
                    b"dependency" if (in_dependencies || in_dep_mgmt) && !in_plugins => {
                        in_dependency = true;
                        cur_group = None;
                        cur_artifact = None;
                        cur_version = None;
                        cur_version_span = None;
                        cur_scope = None;
                        cur_optional = false;
                    }
                    _ => {}
                }
                current_tag = Some(name);
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"dependencies" => in_dependencies = false,
                    b"dependencyManagement" => in_dep_mgmt = false,
                    b"plugins" | b"pluginManagement" => in_plugins = false,
                    b"dependency" if in_dependency => {
                        in_dependency = false;
                        if let (Some(g), Some(a)) = (cur_group.take(), cur_artifact.take()) {
                            let raw_version = cur_version.take().unwrap_or_default();
                            let version = substitute(&raw_version, properties);
                            let scope = cur_scope.take().unwrap_or_default();
                            let dev = scope == "test" || scope == "provided";

                            let (line, version_start, version_end) = match cur_version_span.take() {
                                Some((s, e_)) => {
                                    let (l, col_s) = offset_to_position(&offsets, s);
                                    let (_, col_e) = offset_to_position(&offsets, e_);
                                    (l, col_s, col_e)
                                }
                                None => (0, 0, 0),
                            };

                            if !a.is_empty() && !g.is_empty() {
                                out.push(Dependency {
                                    name: format!("{g}:{a}"),
                                    version,
                                    line,
                                    name_start: 0,
                                    name_end: 0,
                                    version_start,
                                    version_end,
                                    dev,
                                    optional: cur_optional,
                                    registry: None,
                                    resolved_version: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
                current_tag = None;
            }
            Ok(Event::Text(e)) => {
                if in_dependency {
                    let text = match e.decode() {
                        Ok(s) => s.into_owned(),
                        Err(_) => continue,
                    };
                    match current_tag.as_deref() {
                        Some(b"groupId") => cur_group = Some(text),
                        Some(b"artifactId") => cur_artifact = Some(text),
                        Some(b"version") => {
                            // Capture byte offsets of the text content.
                            let end = reader.buffer_position() as usize;
                            let start = end.saturating_sub(text.len());
                            cur_version_span = Some((start, end));
                            cur_version = Some(text);
                        }
                        Some(b"scope") => cur_scope = Some(text),
                        Some(b"optional") => cur_optional = text.trim() == "true",
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let _ = properties; // properties used in substitute(); silence if empty
    out
}

/// Substitute `${property}` placeholders in a version string with values from `properties`.
/// Unresolved placeholders are preserved verbatim.
fn substitute(raw: &str, properties: &HashMap<String, String>) -> String {
    if !raw.contains("${") || properties.is_empty() {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            let key = &after[..end];
            match properties.get(key) {
                Some(v) => out.push_str(v),
                None => {
                    // Preserve the original `${key}` placeholder.
                    out.push_str("${");
                    out.push_str(key);
                    out.push('}');
                }
            }
            rest = &after[end + 1..];
        } else {
            // Unterminated `${`; bail out as literal.
            out.push_str("${");
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_dependency() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>app</artifactId>
    <version>1.0.0</version>
    <dependencies>
        <dependency>
            <groupId>org.slf4j</groupId>
            <artifactId>slf4j-api</artifactId>
            <version>1.7.30</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1, "should parse one dependency");
        assert_eq!(deps[0].name, "org.slf4j:slf4j-api");
        assert_eq!(deps[0].version, "1.7.30");
        assert!(!deps[0].dev);
        assert!(!deps[0].optional);
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd dependi-lsp && cargo test --lib parsers::maven::tests::test_parse_simple_dependency`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add dependi-lsp/src/parsers/mod.rs dependi-lsp/src/parsers/maven.rs
git commit -m "feat(parsers): add MavenParser skeleton with simple dependency parsing"
```

---

## Task 7: Add `<properties>` substitution to MavenParser

**Files:**
- Modify: `dependi-lsp/src/parsers/maven.rs` — replace `extract_properties` body + add test

- [ ] **Step 1: Write failing test**

Add inside `#[cfg(test)] mod tests` in `dependi-lsp/src/parsers/maven.rs`:

```rust
    #[test]
    fn test_parse_with_properties() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>app</artifactId>
    <version>1.0.0</version>
    <properties>
        <spring.version>6.1.0</spring.version>
    </properties>
    <dependencies>
        <dependency>
            <groupId>org.springframework</groupId>
            <artifactId>spring-core</artifactId>
            <version>${spring.version}</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "6.1.0");
    }

    #[test]
    fn test_parse_unresolved_property_preserved() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>${not.defined}</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "${not.defined}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd dependi-lsp && cargo test --lib parsers::maven::tests::test_parse_with_properties`
Expected: FAIL — `deps[0].version` is `${spring.version}`, expected `6.1.0`.

- [ ] **Step 3: Implement extract_properties**

Replace the body of `extract_properties` in `dependi-lsp/src/parsers/maven.rs` with a streaming XML parse that collects children of `<properties>`:

```rust
/// Pass 1: collect `<properties>` entries (name → value) from the pom.
fn extract_properties(content: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut out = HashMap::new();
    let mut depth_stack: Vec<Vec<u8>> = Vec::new();
    let mut current_key: Option<String> = None;

    loop {
        match reader.read_event() {
            Err(_) => return HashMap::new(),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                // We want properties that are a direct child of <properties>
                // which is itself a child of <project>. Path: project > properties > <key>.
                // depth_stack represents the path of open elements excluding the current.
                let is_key = depth_stack.last().map(|v| v.as_slice()) == Some(b"properties");
                if is_key {
                    // Only accept if the grandparent is project.
                    if depth_stack.len() >= 2
                        && depth_stack[depth_stack.len() - 2] == b"project"
                    {
                        if let Ok(s) = std::str::from_utf8(&name) {
                            current_key = Some(s.to_string());
                        }
                    }
                }
                depth_stack.push(name);
            }
            Ok(Event::Text(e)) => {
                if let Some(ref key) = current_key {
                    if let Ok(text) = e.decode() {
                        out.insert(key.clone(), text.into_owned());
                    }
                }
            }
            Ok(Event::End(_)) => {
                depth_stack.pop();
                current_key = None;
            }
            _ => {}
        }
    }

    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd dependi-lsp && cargo test --lib parsers::maven::tests`
Expected: All three tests PASS (simple + with_properties + unresolved_preserved).

- [ ] **Step 5: Commit**

```bash
git add dependi-lsp/src/parsers/maven.rs
git commit -m "feat(parsers): add <properties> substitution in MavenParser"
```

---

## Task 8: Cover remaining parser cases (scope, optional, depMgmt, ignore plugins, invalid XML, position tracking)

**Files:**
- Modify: `dependi-lsp/src/parsers/maven.rs` (add tests only — implementation already covers these)

- [ ] **Step 1: Add comprehensive tests**

Add inside `#[cfg(test)] mod tests` in `dependi-lsp/src/parsers/maven.rs`:

```rust
    #[test]
    fn test_parse_scope_test_marked_as_dev() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>junit</groupId>
            <artifactId>junit</artifactId>
            <version>4.13.2</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].dev);
    }

    #[test]
    fn test_parse_scope_provided_marked_as_dev() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>javax.servlet</groupId>
            <artifactId>servlet-api</artifactId>
            <version>2.5</version>
            <scope>provided</scope>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].dev);
    }

    #[test]
    fn test_parse_optional() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>1.0</version>
            <optional>true</optional>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].optional);
    }

    #[test]
    fn test_parse_snapshot_version() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>2.0-SNAPSHOT</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "2.0-SNAPSHOT");
    }

    #[test]
    fn test_parse_dependency_management() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencyManagement>
        <dependencies>
            <dependency>
                <groupId>g</groupId>
                <artifactId>a</artifactId>
                <version>3.0</version>
            </dependency>
        </dependencies>
    </dependencyManagement>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1, "depMgmt deps with versions should be parsed");
        assert_eq!(deps[0].version, "3.0");
    }

    #[test]
    fn test_parse_plugin_dependencies_ignored() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <build>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-compiler-plugin</artifactId>
                <version>3.11.0</version>
                <dependencies>
                    <dependency>
                        <groupId>ignored</groupId>
                        <artifactId>ignored</artifactId>
                        <version>0.1</version>
                    </dependency>
                </dependencies>
            </plugin>
        </plugins>
    </build>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>1.0</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        // Only the top-level <dependencies>/<dependency> should be captured.
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "g:a");
    }

    #[test]
    fn test_parse_invalid_xml_returns_empty() {
        let parser = MavenParser::new();
        let bad = "<project><dependencies><dependency>";
        let deps = parser.parse(bad);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_position_tracking() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>1.2.3</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        // The version line should be zero-indexed; the exact line varies with the raw string,
        // so we just sanity-check it is non-zero and the span is reasonable.
        assert!(deps[0].line > 0, "line should be tracked (got {})", deps[0].line);
        assert!(
            deps[0].version_end > deps[0].version_start,
            "version span should be non-empty"
        );
    }
```

- [ ] **Step 2: Run tests**

Run: `cd dependi-lsp && cargo test --lib parsers::maven::tests`
Expected: All new tests PASS (along with the earlier 3). If `test_parse_plugin_dependencies_ignored` fails, there is a bug in the plugin state tracking — revisit `extract_dependencies` to ensure `in_plugins` forces `in_dependency` to stay false. If `test_parse_dependency_management` fails, ensure the `dependency` Start arm accepts `in_dep_mgmt`.

- [ ] **Step 3: Commit**

```bash
git add dependi-lsp/src/parsers/maven.rs
git commit -m "test(parsers): cover Maven scope/optional/depMgmt/plugin/invalid cases"
```

---

## Task 9: Create `MavenCentralRegistry` with `maven-metadata.xml` parsing

**Files:**
- Create: `dependi-lsp/src/registries/maven_central.rs`
- Modify: `dependi-lsp/src/registries/mod.rs:228` (add `pub mod maven_central;` after the existing module list)

- [ ] **Step 1: Register module**

Add to `dependi-lsp/src/registries/mod.rs` after line 228 (`pub mod rubygems;`):

```rust
pub mod maven_central;
```

- [ ] **Step 2: Create maven_central.rs with failing tests**

Create `dependi-lsp/src/registries/maven_central.rs`:

```rust
//! # Maven Central Registry Client
//!
//! Fetches version and metadata information for Java packages from
//! [Maven Central](https://repo1.maven.org/maven2) (or a configured mirror).
//!
//! ## Strategy
//!
//! 1. `GET {base_url}/{groupPath}/{artifactId}/maven-metadata.xml` → version list.
//! 2. Best-effort: `GET {base_url}/{groupPath}/{artifactId}/{latest}/{artifactId}-{latest}.pom`
//!    to enrich `VersionInfo` with description, homepage, repository, and license.
//!
//! The second request is non-blocking: on failure the registry returns a partial
//! `VersionInfo` rather than an error.
//!
//! ## Coordinates
//!
//! `package_name` uses the Maven convention `groupId:artifactId` (e.g.
//! `org.slf4j:slf4j-api`). The `groupId` is converted to a path by replacing
//! `.` with `/`.

use std::sync::Arc;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::Client;

use crate::config::MavenRegistryConfig;

use super::{Registry, VersionInfo};

/// Client for Maven Central (or a compatible Maven repository mirror).
pub struct MavenCentralRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl MavenCentralRegistry {
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://repo1.maven.org/maven2".to_string(),
        }
    }

    pub fn with_client_and_config(client: Arc<Client>, config: &MavenRegistryConfig) -> Self {
        let trimmed = config.url.trim_end_matches('/').to_string();
        Self {
            client,
            base_url: trimmed,
        }
    }

    fn coord_path(package_name: &str) -> anyhow::Result<(String, String)> {
        let (group, artifact) = package_name
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!(
                "Invalid Maven coordinate '{package_name}' (expected 'groupId:artifactId')"
            ))?;
        if group.is_empty() || artifact.is_empty() {
            anyhow::bail!(
                "Invalid Maven coordinate '{package_name}' (groupId or artifactId empty)"
            );
        }
        let group_path = group.replace('.', "/");
        Ok((group_path, artifact.to_string()))
    }
}

impl Registry for MavenCentralRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        let (group_path, artifact) = Self::coord_path(package_name)?;

        // Step 1: maven-metadata.xml
        let metadata_url = format!(
            "{}/{}/{}/maven-metadata.xml",
            self.base_url, group_path, artifact
        );
        let resp = self.client.get(&metadata_url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Maven metadata fetch for '{package_name}' failed: HTTP {}",
                resp.status()
            );
        }
        let metadata_body = resp.text().await?;
        let (latest, latest_release, versions) = parse_metadata_xml(&metadata_body)
            .ok_or_else(|| anyhow::anyhow!("Invalid Maven metadata XML for '{package_name}'"))?;

        // Split releases vs prereleases.
        let (stable, prerelease): (Vec<_>, Vec<_>) =
            versions.iter().cloned().partition(|v| !is_prerelease(v));

        // Preferred latest stable: <release> if present, else highest stable.
        let latest_stable = latest_release
            .or_else(|| latest.clone())
            .or_else(|| stable.first().cloned());

        // Prerelease: first in the raw order if any.
        let latest_prerelease = prerelease.first().cloned();

        // Step 2: best-effort POM fetch for metadata (description, license, ...)
        let (description, homepage, repository, license) = match &latest_stable {
            Some(v) => {
                let pom_url = format!(
                    "{}/{}/{}/{}/{}-{}.pom",
                    self.base_url, group_path, artifact, v, artifact, v
                );
                match self.client.get(&pom_url).send().await {
                    Ok(r) if r.status().is_success() => match r.text().await {
                        Ok(body) => parse_pom_metadata(&body),
                        Err(e) => {
                            tracing::debug!(
                                "Maven POM text read failed for {package_name}@{v}: {e}"
                            );
                            (None, None, None, None)
                        }
                    },
                    Ok(r) => {
                        tracing::debug!(
                            "Maven POM fetch for {package_name}@{v} returned HTTP {}",
                            r.status()
                        );
                        (None, None, None, None)
                    }
                    Err(e) => {
                        tracing::debug!("Maven POM fetch for {package_name}@{v} failed: {e}");
                        (None, None, None, None)
                    }
                }
            }
            None => (None, None, None, None),
        };

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions,
            description,
            homepage,
            repository,
            license,
            vulnerabilities: vec![],
            deprecated: false,
            yanked: false,
            yanked_versions: vec![],
            release_dates: hashbrown::HashMap::new(),
        })
    }

    fn http_client(&self) -> Arc<Client> {
        self.client.clone()
    }
}

/// Parse `maven-metadata.xml` → (latest, release, versions[] in descending order).
/// `versions` preserves document order reversed (newest first as Maven writes them last).
pub(crate) fn parse_metadata_xml(content: &str) -> Option<(Option<String>, Option<String>, Vec<String>)> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut latest: Option<String> = None;
    let mut release: Option<String> = None;
    let mut versions: Vec<String> = Vec::new();

    let mut stack: Vec<Vec<u8>> = Vec::new();

    loop {
        match reader.read_event() {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => stack.push(e.name().as_ref().to_vec()),
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Text(e)) => {
                let text = match e.decode() {
                    Ok(s) => s.into_owned(),
                    Err(_) => continue,
                };
                // Path checks: metadata > versioning > latest | release
                // Path: metadata > versioning > versions > version
                let len = stack.len();
                if len >= 3
                    && stack[len - 3] == b"metadata"
                    && stack[len - 2] == b"versioning"
                {
                    match stack[len - 1].as_slice() {
                        b"latest" => latest = Some(text),
                        b"release" => release = Some(text),
                        _ => {}
                    }
                } else if len >= 4
                    && stack[len - 4] == b"metadata"
                    && stack[len - 3] == b"versioning"
                    && stack[len - 2] == b"versions"
                    && stack[len - 1] == b"version"
                {
                    versions.push(text);
                }
            }
            _ => {}
        }
    }

    // Newest-first ordering: Maven writes versions in ascending order.
    versions.reverse();

    Some((latest, release, versions))
}

/// Parse a minimal subset of a pom.xml to extract presentation metadata.
pub(crate) fn parse_pom_metadata(content: &str) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut description: Option<String> = None;
    let mut homepage: Option<String> = None;
    let mut repository: Option<String> = None;
    let mut licenses: Vec<String> = Vec::new();

    let mut stack: Vec<Vec<u8>> = Vec::new();

    loop {
        match reader.read_event() {
            Err(_) => break,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => stack.push(e.name().as_ref().to_vec()),
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Text(e)) => {
                let text = match e.decode() {
                    Ok(s) => s.into_owned(),
                    Err(_) => continue,
                };
                let len = stack.len();
                // project > description
                if len == 2 && stack[0] == b"project" && stack[1] == b"description" {
                    description = Some(text);
                    continue;
                }
                // project > url
                if len == 2 && stack[0] == b"project" && stack[1] == b"url" {
                    homepage = Some(text);
                    continue;
                }
                // project > scm > url
                if len == 3
                    && stack[0] == b"project"
                    && stack[1] == b"scm"
                    && stack[2] == b"url"
                {
                    repository = Some(text);
                    continue;
                }
                // project > licenses > license > name
                if len == 4
                    && stack[0] == b"project"
                    && stack[1] == b"licenses"
                    && stack[2] == b"license"
                    && stack[3] == b"name"
                {
                    licenses.push(text);
                }
            }
            _ => {}
        }
    }

    let license = if licenses.is_empty() {
        None
    } else {
        Some(licenses.join(", "))
    };
    (description, homepage, repository, license)
}

/// Classify a Maven version string as a prerelease / snapshot.
fn is_prerelease(version: &str) -> bool {
    let v = version.to_ascii_lowercase();
    v.contains("-snapshot")
        || v.contains("-alpha")
        || v.contains("-beta")
        || v.contains("-rc")
        || v.contains("-m")
        || v.contains("-milestone")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coord_path_ok() {
        let (g, a) = MavenCentralRegistry::coord_path("org.slf4j:slf4j-api").unwrap();
        assert_eq!(g, "org/slf4j");
        assert_eq!(a, "slf4j-api");
    }

    #[test]
    fn test_coord_path_invalid() {
        assert!(MavenCentralRegistry::coord_path("no-colon").is_err());
        assert!(MavenCentralRegistry::coord_path(":empty").is_err());
        assert!(MavenCentralRegistry::coord_path("empty:").is_err());
    }

    #[test]
    fn test_parse_metadata_xml_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>org.slf4j</groupId>
  <artifactId>slf4j-api</artifactId>
  <versioning>
    <latest>2.0.9</latest>
    <release>2.0.9</release>
    <versions>
      <version>1.7.30</version>
      <version>2.0.0</version>
      <version>2.0.9</version>
    </versions>
  </versioning>
</metadata>
"#;
        let (latest, release, versions) = parse_metadata_xml(xml).expect("parse ok");
        assert_eq!(latest.as_deref(), Some("2.0.9"));
        assert_eq!(release.as_deref(), Some("2.0.9"));
        // Newest first
        assert_eq!(versions, vec!["2.0.9", "2.0.0", "1.7.30"]);
    }

    #[test]
    fn test_parse_pom_extracts_description_and_license() {
        let pom = r#"<?xml version="1.0"?>
<project>
    <description>Structured logging API</description>
    <url>https://example.com</url>
    <scm>
        <url>https://github.com/example/example</url>
    </scm>
    <licenses>
        <license>
            <name>Apache-2.0</name>
        </license>
    </licenses>
</project>
"#;
        let (description, homepage, repository, license) = parse_pom_metadata(pom);
        assert_eq!(description.as_deref(), Some("Structured logging API"));
        assert_eq!(homepage.as_deref(), Some("https://example.com"));
        assert_eq!(
            repository.as_deref(),
            Some("https://github.com/example/example")
        );
        assert_eq!(license.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn test_parse_pom_missing_license_returns_none() {
        let pom = "<project><description>no license</description></project>";
        let (_description, _homepage, _repository, license) = parse_pom_metadata(pom);
        assert_eq!(license, None);
    }

    #[test]
    fn test_parse_pom_multiple_licenses_joined() {
        let pom = r#"<project>
    <licenses>
        <license><name>Apache-2.0</name></license>
        <license><name>MIT</name></license>
    </licenses>
</project>"#;
        let (_, _, _, license) = parse_pom_metadata(pom);
        assert_eq!(license.as_deref(), Some("Apache-2.0, MIT"));
    }

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0-SNAPSHOT"));
        assert!(is_prerelease("1.0-alpha-1"));
        assert!(is_prerelease("2.0-rc1"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.5.1"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd dependi-lsp && cargo test --lib registries::maven_central::tests`
Expected: All 7 tests PASS.

- [ ] **Step 4: Update registries/mod.rs doc table**

In `dependi-lsp/src/registries/mod.rs`, modify the documentation table (around line 17) to add Maven Central. Replace the existing table with:

```rust
//! | Registry | Ecosystem | Module |
//! |----------|-----------|--------|
//! | [crates.io](https://crates.io) | Rust | [`crates_io`] |
//! | [npm](https://www.npmjs.com) | Node.js/JavaScript | [`npm`] |
//! | [PyPI](https://pypi.org) | Python | [`pypi`] |
//! | [Go Proxy](https://proxy.golang.org) | Go | [`go_proxy`] |
//! | [Packagist](https://packagist.org) | PHP/Composer | [`packagist`] |
//! | [pub.dev](https://pub.dev) | Dart/Flutter | [`pub_dev`] |
//! | [NuGet](https://www.nuget.org) | .NET | [`nuget`] |
//! | [RubyGems](https://rubygems.org) | Ruby | [`rubygems`] |
//! | [Maven Central](https://repo1.maven.org/maven2) | Java/Maven | [`maven_central`] |
```

- [ ] **Step 5: Commit**

```bash
git add dependi-lsp/src/registries/mod.rs dependi-lsp/src/registries/maven_central.rs
git commit -m "feat(registries): add MavenCentralRegistry with metadata+POM parsing"
```

---

## Task 10: Wire MavenParser and MavenCentralRegistry into backend

**Files:**
- Modify: `dependi-lsp/src/backend.rs:28` (imports)
- Modify: `dependi-lsp/src/backend.rs:68-92` (`ProcessingContext` struct)
- Modify: `dependi-lsp/src/backend.rs:95-107` (`ProcessingContext::parse_document`)
- Modify: `dependi-lsp/src/backend.rs:454-475` (fetch loop match arm)
- Modify: `dependi-lsp/src/backend.rs:594-637` (`DependiBackend` struct)
- Modify: `dependi-lsp/src/backend.rs:670-700` (`with_http_client` init)
- Modify: `dependi-lsp/src/backend.rs:702-728` (`create_processing_context`)
- Modify: `dependi-lsp/src/backend.rs:749-775` (`get_version_info` match arm)

- [ ] **Step 1: Add imports**

In `dependi-lsp/src/backend.rs`, add imports after the existing parser imports (around line 27):

```rust
use crate::parsers::maven::MavenParser;
```

And after the existing registry imports (around line 42):

```rust
use crate::registries::maven_central::MavenCentralRegistry;
```

- [ ] **Step 2: Add `maven_parser` and `maven_central` fields to `ProcessingContext`**

In `dependi-lsp/src/backend.rs`, modify `ProcessingContext` (lines 67-92). Add `maven_parser` after `ruby_parser` and `maven_central` after `rubygems`:

```rust
#[derive(Clone)]
struct ProcessingContext {
    client: Client,
    config: Arc<RwLock<Config>>,
    documents: Arc<DashMap<Url, DocumentState>>,
    version_cache: Arc<HybridCache>,
    cargo_parser: Arc<CargoParser>,
    npm_parser: Arc<NpmParser>,
    python_parser: Arc<PythonParser>,
    go_parser: Arc<GoParser>,
    php_parser: Arc<PhpParser>,
    dart_parser: Arc<DartParser>,
    csharp_parser: Arc<CsharpParser>,
    ruby_parser: Arc<RubyParser>,
    maven_parser: Arc<MavenParser>,
    crates_io: Arc<CratesIoRegistry>,
    cargo_custom_registries: Arc<DashMap<String, Arc<CargoSparseRegistry>>>,
    npm_registry: Arc<tokio::sync::RwLock<NpmRegistry>>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
    pub_dev: Arc<PubDevRegistry>,
    nuget: Arc<NuGetRegistry>,
    rubygems: Arc<RubyGemsRegistry>,
    maven_central: Arc<MavenCentralRegistry>,
    osv_client: Arc<OsvClient>,
    vuln_cache: Arc<VulnerabilityCache>,
}
```

- [ ] **Step 3: Add match arm in `ProcessingContext::parse_document`**

Modify the match in `parse_document` (lines 96-106) to add a Maven arm (and remove the catch-all `None => vec![]` behavior stays):

```rust
    fn parse_document(&self, uri: &Url, content: &str) -> Vec<crate::parsers::Dependency> {
        match FileType::detect(uri) {
            Some(FileType::Cargo) => self.cargo_parser.parse(content),
            Some(FileType::Npm) => self.npm_parser.parse(content),
            Some(FileType::Python) => self.python_parser.parse(content),
            Some(FileType::Go) => self.go_parser.parse(content),
            Some(FileType::Php) => self.php_parser.parse(content),
            Some(FileType::Dart) => self.dart_parser.parse(content),
            Some(FileType::Csharp) => self.csharp_parser.parse(content),
            Some(FileType::Ruby) => self.ruby_parser.parse(content),
            Some(FileType::Maven) => self.maven_parser.parse(content),
            None => vec![],
        }
    }
```

- [ ] **Step 4: Add match arm in the fetch loop**

Modify the fetch match (lines 454-475). First, clone `maven_central` into the task closure. Around line 444 (after `let rubygems = Arc::clone(&rubygems);`), add:

```rust
                let maven_central = Arc::clone(&maven_central);
```

Then in the outer scope (around line 426, where `let rubygems = Arc::clone(&self.rubygems);` probably lives — scan backwards to find where the registries are bound from `self` before the `.map(|dep|` closure), add the binding. If it's inside `process_document`, grep for `let rubygems = Arc::clone` to confirm the location — insert immediately after it:

```rust
        let maven_central = Arc::clone(&self.maven_central);
```

Finally, in the match (line 454) add the Maven arm after `Ruby`:

```rust
                        FileType::Ruby => rubygems.get_version_info(&name).await,
                        FileType::Maven => maven_central.get_version_info(&name).await,
                    };
```

- [ ] **Step 5: Add `maven_parser` and `maven_central` fields to `DependiBackend`**

Modify `DependiBackend` struct (lines 594-637). Add after `ruby_parser: Arc<RubyParser>,`:

```rust
    maven_parser: Arc<MavenParser>,
```

Add after `rubygems: Arc<RubyGemsRegistry>,`:

```rust
    maven_central: Arc<MavenCentralRegistry>,
```

- [ ] **Step 6: Initialize in `with_http_client`**

In `with_http_client` (lines 657-700), after line 682 (`ruby_parser: Arc::new(RubyParser::new()),`), add:

```rust
            maven_parser: Arc::new(MavenParser::new()),
```

After line 691 (`rubygems: Arc::new(RubyGemsRegistry::with_client(Arc::clone(&http_client))),`), add:

```rust
            maven_central: Arc::new(MavenCentralRegistry::with_client_and_config(
                Arc::clone(&http_client),
                &config.registries.maven,
            )),
```

- [ ] **Step 7: Clone in `create_processing_context`**

In `create_processing_context` (lines 702-728), after `ruby_parser: Arc::clone(&self.ruby_parser),` add:

```rust
            maven_parser: Arc::clone(&self.maven_parser),
```

After `rubygems: Arc::clone(&self.rubygems),` add:

```rust
            maven_central: Arc::clone(&self.maven_central),
```

- [ ] **Step 8: Add match arm in `get_version_info`**

In `get_version_info` (lines 749-774), add a Maven arm after `Ruby`:

```rust
            FileType::Ruby => self.rubygems.get_version_info(package_name).await,
            FileType::Maven => self.maven_central.get_version_info(package_name).await,
```

- [ ] **Step 9: Build the whole library**

Run: `cd dependi-lsp && cargo build --lib`
Expected: Clean build, no errors, no new warnings.

If missing match arms remain, grep will find them:

```bash
cd /home/matvei/projets/zed-dependi && rg "FileType::Ruby" dependi-lsp/src --files-with-matches
```

For each match, ensure a `FileType::Maven` arm exists adjacent to it.

- [ ] **Step 10: Run full test suite**

Run: `cd dependi-lsp && cargo test --lib`
Expected: All tests PASS (existing + new Maven tests).

- [ ] **Step 11: Commit**

```bash
git add dependi-lsp/src/backend.rs
git commit -m "feat(backend): wire MavenParser and MavenCentralRegistry end-to-end"
```

---

## Task 11: Lint and format checks

**Files:**
- No file changes — verification only.

- [ ] **Step 1: Run clippy**

Run: `cd dependi-lsp && cargo clippy --all-targets -- -D warnings`
Expected: No warnings, no errors.

If there are warnings, fix them minimally in the corresponding Maven files (avoid touching unrelated code). Common issues:
- `uninlined_format_args` → convert `format!("{}", x)` to `format!("{x}")`
- `needless_borrow` → remove `&`
- Unused imports → remove

- [ ] **Step 2: Run fmt check**

Run: `cd dependi-lsp && cargo fmt --all -- --check`
Expected: No output (all files properly formatted).

If it complains, apply formatting: `cd dependi-lsp && cargo fmt --all`

- [ ] **Step 3: Commit any fixes**

```bash
git add -A
git diff --cached --stat
git commit -m "style(maven): apply clippy and rustfmt suggestions"
```

If there are no changes (all clean), skip the commit.

---

## Task 12: Integration test for pom.xml end-to-end

**Files:**
- Modify: `dependi-lsp/tests/integration_test.rs` (add new test at the end)

- [ ] **Step 1: Inspect existing tests**

Run: `cd dependi-lsp && head -50 tests/integration_test.rs`

Confirm the style: what APIs are imported, how they build a `DependiBackend` or similar. If no relevant harness exists, add a smaller scope test that parses a `pom.xml` and asserts detection + basic parse.

- [ ] **Step 2: Add test**

At the end of `dependi-lsp/tests/integration_test.rs`, add:

```rust
#[test]
fn maven_pom_detection_and_parse() {
    use dependi_lsp::file_types::FileType;
    use dependi_lsp::parsers::{Parser, maven::MavenParser};
    use tower_lsp::lsp_types::Url;

    let uri = Url::parse("file:///project/pom.xml").unwrap();
    assert_eq!(FileType::detect(&uri), Some(FileType::Maven));

    let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>org.slf4j</groupId>
            <artifactId>slf4j-api</artifactId>
            <version>1.7.30</version>
        </dependency>
    </dependencies>
</project>
"#;
    let parser = MavenParser::new();
    let deps = parser.parse(pom);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].name, "org.slf4j:slf4j-api");
    assert_eq!(deps[0].version, "1.7.30");
    assert_eq!(FileType::Maven.cache_key(&deps[0].name), "maven:org.slf4j:slf4j-api");
}
```

- [ ] **Step 3: Run integration tests**

Run: `cd dependi-lsp && cargo test --test integration_test maven_pom_detection_and_parse`
Expected: PASS. If any `use` path is wrong (e.g., `MavenParser` not re-exported), adjust by reading `dependi-lsp/src/lib.rs` and using whatever path other parsers use in existing integration tests.

- [ ] **Step 4: Run all integration tests**

Run: `cd dependi-lsp && cargo test --test integration_test`
Expected: All tests PASS.

- [ ] **Step 5: Commit**

```bash
git add dependi-lsp/tests/integration_test.rs
git commit -m "test(integration): add end-to-end Maven pom.xml detection+parse"
```

---

## Task 13: Update CHANGELOG.md

**Files:**
- Modify: `CHANGELOG.md` (add entry under `[Unreleased]` → `### Added`)

- [ ] **Step 1: Read current CHANGELOG**

Run: `head -30 /home/matvei/projets/zed-dependi/CHANGELOG.md`

Locate the `[Unreleased]` section. If `### Added` already exists, append to it. If not, create it.

- [ ] **Step 2: Add the Maven entry**

In `CHANGELOG.md`, add under `[Unreleased]` → `### Added`:

```markdown
- Support for Java/Maven projects (pom.xml):
  - Parse direct dependencies with `${properties}` substitution
  - Scope awareness (`test`/`provided` marked as dev dependencies)
  - Fetch versions and metadata from Maven Central (`maven-metadata.xml` + best-effort POM)
  - Vulnerability scanning via OSV.dev (Maven ecosystem)
  - Configurable base URL for alternative Maven repositories (Nexus/Artifactory mirrors)
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): add entry for Maven/pom.xml support (#223)"
```

---

## Task 14: Full verification before PR

**Files:**
- No file changes — verification only.

- [ ] **Step 1: Build release**

Run: `cd dependi-lsp && cargo build --release --package dependi-lsp`
Expected: Clean release build.

- [ ] **Step 2: Run all tests**

Run: `cd dependi-lsp && cargo test --lib && cargo test --test integration_test`
Expected: All tests pass, no failures.

- [ ] **Step 3: Run clippy + fmt**

Run:
```bash
cd dependi-lsp && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check
```
Expected: No errors, no output from `--check`.

- [ ] **Step 4: Check there are no pending files**

Run: `cd /home/matvei/projets/zed-dependi && git status`
Expected: `working tree clean` (all work committed).

- [ ] **Step 5: Adversarial review checkpoint**

The implementation is now ready for adversarial review (see the separate review step in the parent workflow). Do not create the PR yet — wait for review findings to be addressed.

---

## Self-Review — plan vs. spec

**Spec coverage:**
- ✅ Architecture & 15 integration points — Tasks 2, 3, 4, 5, 10
- ✅ MavenParser with 2-pass (properties, dependencies) — Tasks 6, 7
- ✅ Scope awareness, optional, dependencyManagement, plugins ignored, invalid XML, position tracking — Task 8
- ✅ MavenCentralRegistry (metadata + best-effort POM) — Task 9
- ✅ Config `MavenRegistryConfig` with configurable URL — Task 5
- ✅ Backend integration — Task 10
- ✅ No lockfile resolution — explicitly skipped in Task 10
- ✅ No auth — confirmed; config only has `url`
- ✅ Tests inline, integration test — Tasks 6-9, 12
- ✅ CHANGELOG entry — Task 13
- ✅ code_actions fallback — already exists at `providers/code_actions.rs:56` (no change needed)
- ✅ Parent POM — not resolved (spec: MVP, detected but NOOP) — no code needed; the `<parent>` tag is simply not processed

**Type consistency check:**
- `MavenParser::new()` — referenced in Task 6 (definition) and Task 10 (usage) ✅
- `MavenCentralRegistry::with_client_and_config(client, &config)` — Task 9 defines, Task 10 uses ✅
- `MavenRegistryConfig { url: String }` — Task 5 defines, Task 10 reads `config.registries.maven` ✅
- `FileType::Maven`, `Ecosystem::Maven` — defined in Tasks 2-3, consumed in 4, 10 ✅
- `parse_metadata_xml`, `parse_pom_metadata`, `is_prerelease` — all defined in Task 9 ✅

**Placeholder scan:** None. All code is complete.

**No catch-all arms:** Match statements on `FileType` in `backend.rs` already enumerate explicitly — Task 10 adds Maven arms everywhere.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-17-maven-pom-xml-support.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks
2. **Inline Execution** — execute tasks in this session with checkpoints

The user's original request asked for parallel agent execution during implementation and adversarial review at the end. This plan will be executed by combining parallel subagent dispatch per file-scoped task where possible (parser + registry are independent), with sequential integration for the cross-cutting backend wiring.
