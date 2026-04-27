# npm/PHP JSON spans — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace string-search position tracking in `npm.rs` and `php.rs` with span information from a span-aware JSON parser, factoring shared helpers into `parsers/json_spans.rs`.

**Architecture:** Add `json-spanned-value` 0.2.2 as a dependency. Create a shared `LineIndex` + span-conversion helper module. Rewrite `NpmParser::parse` and `PhpParser::parse` to deserialize into `spanned::Object` and read byte spans directly from the parser, replacing `compute_line_offsets` + `find_dependency_position`.

**Tech Stack:** Rust (edition 2024, MSRV 1.94), `json-spanned-value` 0.2.2, `serde_json` (already present), Criterion benches, TDD.

**Spec:** [`docs/specs/2026-04-27-npm-php-json-spans-design.md`](../2026-04-27-npm-php-json-spans-design.md)

**Issue:** [#236](https://github.com/mpiton/zed-dependi/issues/236)

---

## File structure

| Path | Action | Purpose |
|---|---|---|
| `dependi-lsp/Cargo.toml` | Modify | +1 dependency |
| `dependi-lsp/src/parsers/mod.rs` | Modify | +`pub mod json_spans;` |
| `dependi-lsp/src/parsers/json_spans.rs` | Create | `LineIndex` + helpers + tests |
| `dependi-lsp/src/parsers/npm.rs` | Rewrite | Use spans, drop string search |
| `dependi-lsp/src/parsers/php.rs` | Rewrite | Use spans, drop string search |
| `CHANGELOG.md` | Modify | Entry under `[Unreleased]` |

---

## Parallelism notes

- **Task 1 → Task 2**: hard dependency (json_spans must exist before npm/php use it).
- **Task 2 (npm) and Task 3 (php)**: independent, can run in parallel once Task 1 is merged.
- **Task 4 (changelog)**: serial, after 2 + 3.

---

## Task 1: Add dependency and create `json_spans` helper module

**Files:**
- Modify: `dependi-lsp/Cargo.toml`
- Modify: `dependi-lsp/src/parsers/mod.rs`
- Create: `dependi-lsp/src/parsers/json_spans.rs`

### Step 1.1: Write failing tests for `LineIndex`

- [ ] **Create `dependi-lsp/src/parsers/json_spans.rs` with the test module only:**

```rust
//! Shared helpers for span-aware JSON parsing (used by npm and php parsers).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_empty_content() {
        let idx = LineIndex::new("");
        assert_eq!(idx.position(0), (0, 0));
    }

    #[test]
    fn line_index_first_line() {
        let idx = LineIndex::new("hello\nworld");
        assert_eq!(idx.position(0), (0, 0));
        assert_eq!(idx.position(4), (0, 4));
    }

    #[test]
    fn line_index_after_newline() {
        let idx = LineIndex::new("hello\nworld");
        assert_eq!(idx.position(6), (1, 0));
        assert_eq!(idx.position(10), (1, 4));
    }

    #[test]
    fn line_index_three_lines() {
        let idx = LineIndex::new("a\nbb\nccc");
        assert_eq!(idx.position(0), (0, 0));
        assert_eq!(idx.position(2), (1, 0));
        assert_eq!(idx.position(5), (2, 0));
        assert_eq!(idx.position(7), (2, 2));
    }

    #[test]
    fn line_index_offset_past_end_clamps_to_last_line() {
        let idx = LineIndex::new("ab");
        let (line, _col) = idx.position(99);
        assert_eq!(line, 0);
    }

    #[test]
    fn line_index_multibyte_utf8_byte_columns() {
        // "é" is 2 bytes in UTF-8. Columns are byte offsets.
        let content = "ab\néd";
        let idx = LineIndex::new(content);
        assert_eq!(idx.position(3), (1, 0)); // 'é' start
        assert_eq!(idx.position(5), (1, 2)); // 'd' (after 2-byte 'é')
    }

    #[test]
    fn span_to_span_single_line() {
        let idx = LineIndex::new("hello world");
        let s = span_to_span(&idx, 6, 11).unwrap();
        assert_eq!(s.line, 0);
        assert_eq!(s.line_start, 6);
        assert_eq!(s.line_end, 11);
    }

    #[test]
    fn span_to_span_multi_line_returns_none() {
        let idx = LineIndex::new("hello\nworld");
        assert!(span_to_span(&idx, 4, 8).is_none());
    }

    #[test]
    fn inner_string_span_strips_quotes() {
        // For a JSON string `"abc"` at bytes 0..5, inner content is bytes 1..4.
        let (s, e) = inner_string_span(0, 5);
        assert_eq!((s, e), (1, 4));
    }

    #[test]
    fn inner_string_span_empty_string() {
        // Empty JSON string `""` at bytes 0..2, inner is 1..1.
        let (s, e) = inner_string_span(0, 2);
        assert_eq!((s, e), (1, 1));
    }
}
```

- [ ] **Run tests to confirm they fail (compile error):**

```bash
cargo test --package dependi-lsp --lib parsers::json_spans 2>&1 | tail -20
```

Expected: compile error (`LineIndex`, `span_to_span`, `inner_string_span` undefined).

### Step 1.2: Implement `LineIndex` + helpers

- [ ] **Replace contents of `dependi-lsp/src/parsers/json_spans.rs`:**

```rust
//! Shared helpers for span-aware JSON parsing (used by npm and php parsers).

use super::Span;

/// Pre-computed byte offsets of each line start in a source string.
///
/// Constructed in O(n); each `position` query is O(log n) via binary search.
#[derive(Debug)]
pub struct LineIndex {
    /// `offsets[i]` is the byte offset where line `i` starts.
    offsets: Vec<usize>,
}

impl LineIndex {
    /// Build a `LineIndex` from `content`. Always contains at least one entry (`0`).
    pub fn new(content: &str) -> Self {
        let mut offsets = Vec::with_capacity(content.len() / 32 + 1);
        offsets.push(0);
        for (i, byte) in content.bytes().enumerate() {
            if byte == b'\n' {
                offsets.push(i + 1);
            }
        }
        Self { offsets }
    }

    /// Convert a byte offset to a `(line, column)` pair, both 0-indexed.
    /// Columns are byte offsets within the line.
    /// Offsets past the end clamp to the last line.
    pub fn position(&self, byte_offset: usize) -> (u32, u32) {
        let line = match self.offsets.binary_search(&byte_offset) {
            Ok(exact) => exact,
            Err(insert_at) => insert_at.saturating_sub(1),
        };
        let col = byte_offset - self.offsets[line];
        (line as u32, col as u32)
    }
}

/// Convert a byte range `[start, end)` to a `Span` if it fits on a single line.
/// Returns `None` if the range straddles a line boundary.
pub fn span_to_span(line_index: &LineIndex, start: usize, end: usize) -> Option<Span> {
    let (line_start, col_start) = line_index.position(start);
    let (line_end, col_end) = line_index.position(end);
    if line_start != line_end {
        return None;
    }
    Some(Span {
        line: line_start,
        line_start: col_start,
        line_end: col_end,
    })
}

/// Strip the surrounding `"…"` from a JSON string's byte range.
/// `(start, end)` are the outer quote-inclusive bounds; the result is the
/// inner content bounds, suitable for `span_to_span`.
pub fn inner_string_span(start: usize, end: usize) -> (usize, usize) {
    // Outer span includes quotes; inner content is one byte in on each side.
    (start + 1, end.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    // (test module from step 1.1 — keep unchanged)
}
```

(Keep the test module already in place from step 1.1.)

- [ ] **Run the tests, expect all to pass:**

```bash
cargo test --package dependi-lsp --lib parsers::json_spans 2>&1 | tail -20
```

Expected: `test result: ok. 9 passed; 0 failed`.

### Step 1.3: Wire module into `parsers/mod.rs`

- [ ] **Edit `dependi-lsp/src/parsers/mod.rs`. After the existing `pub mod cargo;` line (or wherever modules are listed alphabetically), add:**

```rust
pub mod json_spans;
```

Keep alphabetical ordering with the existing modules.

### Step 1.4: Add `json-spanned-value` dependency

- [ ] **Run:**

```bash
cargo add --package dependi-lsp json-spanned-value@0.2.2
```

- [ ] **Verify the line in `dependi-lsp/Cargo.toml` looks like:**

```toml
json-spanned-value = "0.2.2"
```

- [ ] **Build to confirm dependency resolves:**

```bash
cargo build --package dependi-lsp 2>&1 | tail -5
```

Expected: build succeeds (warnings about unused `json-spanned-value` import are acceptable since it is not used yet — but we did not add a `use` statement, so there should be none).

### Step 1.5: Lint and commit

- [ ] **Run lint + format:**

```bash
cargo clippy --package dependi-lsp --all-targets -- -D warnings
cargo fmt --all -- --check
```

Both must succeed.

- [ ] **Commit:**

```bash
git add dependi-lsp/Cargo.toml dependi-lsp/Cargo.lock dependi-lsp/src/parsers/mod.rs dependi-lsp/src/parsers/json_spans.rs
git commit -m "$(cat <<'EOF'
feat(parsers): add LineIndex + span helpers for JSON parsers (#236)

Introduce parsers/json_spans.rs with a shared LineIndex (O(n) build,
O(log n) lookup) and helpers for converting byte spans to the Span
type used by Dependency. Adds json-spanned-value 0.2.2.

Foundation for the npm/php parser refactor; no behaviour change yet.
EOF
)"
```

---

## Task 2: Refactor `npm.rs` to use spans

**Depends on:** Task 1 must be merged.

**Files:**
- Rewrite: `dependi-lsp/src/parsers/npm.rs`

### Step 2.1: Add the new failing edge-case tests

- [ ] **Append to the `mod tests` block in `dependi-lsp/src/parsers/npm.rs` (before the closing `}` of the module):**

```rust
    #[test]
    fn test_same_name_in_two_sections() {
        let parser = NpmParser::new();
        let content = r#"{
  "dependencies": {
    "foo": "1.0.0"
  },
  "devDependencies": {
    "foo": "2.0.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let prod = deps.iter().find(|d| !d.dev).unwrap();
        let dev = deps.iter().find(|d| d.dev).unwrap();
        assert_eq!(prod.version, "1.0.0");
        assert_eq!(dev.version, "2.0.0");
        // Spans must be on different lines (the bug we are fixing: string
        // search may match the same line for both).
        assert_ne!(prod.name_span.line, dev.name_span.line);
    }

    #[test]
    fn test_substring_false_match_in_value() {
        // The "description" field contains a literal that looks like a
        // dependency entry. The parser must not pick it up as a dep.
        let parser = NpmParser::new();
        let content = r#"{
  "description": "looks like \"react\": \"99.0.0\" but is text",
  "dependencies": {
    "react": "1.0.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "1.0.0");
    }

    #[test]
    fn test_whitespace_variations() {
        let parser = NpmParser::new();
        let content = "{\n  \"dependencies\": {\n    \"a\":\t\t\"1.0.0\",\n    \"b\"  :   \"2.0.0\"\n  }\n}";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);
        let a = deps.iter().find(|d| d.name == "a").unwrap();
        let b = deps.iter().find(|d| d.name == "b").unwrap();
        assert_eq!(a.version, "1.0.0");
        assert_eq!(b.version, "2.0.0");
        // Sanity: each version span lies inside its declared line range.
        assert!(a.version_span.line_start < a.version_span.line_end);
        assert!(b.version_span.line_start < b.version_span.line_end);
    }

    #[test]
    fn test_large_file_smoke() {
        let mut content = String::from("{\n  \"dependencies\": {\n");
        for i in 0..1000 {
            let comma = if i == 999 { "" } else { "," };
            content.push_str(&format!("    \"pkg{i}\": \"1.0.{i}\"{comma}\n"));
        }
        content.push_str("  }\n}");
        let parser = NpmParser::new();
        let start = std::time::Instant::now();
        let deps = parser.parse(&content);
        let elapsed = start.elapsed();
        assert_eq!(deps.len(), 1000);
        // Generous bound — purely a smoke check that we are not quadratic.
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "parse took {elapsed:?}"
        );
    }
```

- [ ] **Run the test module, expect the duplicate-name test to fail (and possibly the substring test):**

```bash
cargo test --package dependi-lsp --lib parsers::npm 2>&1 | tail -30
```

Expected: at least `test_same_name_in_two_sections` fails, demonstrating the bug.

### Step 2.2: Rewrite `npm.rs` to use `json-spanned-value`

- [ ] **Replace the entire module contents with the following (keep the test module unchanged at the bottom):**

```rust
//! Parser for package.json files
//!
//! Uses `json-spanned-value` to obtain dependency name/version spans directly
//! from the parser output, removing the need for a manual string scan.

use json_spanned_value as jsv;
use json_spanned_value::spanned;

use super::json_spans::{LineIndex, inner_string_span, span_to_span};
use super::{Dependency, Parser, Span};

/// Parser for npm package.json dependency files.
#[derive(Debug, Default)]
pub struct NpmParser;

impl NpmParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for NpmParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let Ok(root) = jsv::from_str::<spanned::Object>(content) else {
            return Vec::new();
        };

        let line_index = LineIndex::new(content);
        let mut dependencies = Vec::with_capacity(64);

        parse_section(&root, "dependencies", false, false, &line_index, &mut dependencies);
        parse_section(&root, "devDependencies", true, false, &line_index, &mut dependencies);
        parse_section(&root, "peerDependencies", false, true, &line_index, &mut dependencies);
        parse_section(&root, "optionalDependencies", false, true, &line_index, &mut dependencies);

        dependencies
    }
}

/// Look up a section in the root object and parse each entry into a `Dependency`.
fn parse_section(
    root: &spanned::Object,
    section_name: &str,
    dev: bool,
    optional: bool,
    line_index: &LineIndex,
    dependencies: &mut Vec<Dependency>,
) {
    let Some(section_value) = root.get(section_name) else {
        return;
    };
    let Some(section_obj) = section_value.as_object() else {
        return;
    };

    for (name_spanned, value_spanned) in section_obj {
        let name_span = match string_inner_to_span(line_index, name_spanned.start(), name_spanned.end()) {
            Some(s) => s,
            None => continue,
        };

        let Some((version, version_span)) = extract_version(value_spanned, line_index) else {
            continue;
        };

        if name_span.line != version_span.line {
            continue;
        }

        dependencies.push(Dependency {
            name: name_spanned.get_ref().clone(),
            version,
            name_span,
            version_span,
            dev,
            optional,
            registry: None,
            resolved_version: None,
        });
    }
}

/// Convert outer (quote-inclusive) byte bounds of a JSON string to an inner-content `Span`.
fn string_inner_to_span(line_index: &LineIndex, start: usize, end: usize) -> Option<Span> {
    let (inner_start, inner_end) = inner_string_span(start, end);
    span_to_span(line_index, inner_start, inner_end)
}

/// Extract a version string and its inner-content span from a value that is
/// either a JSON string or an object containing `"version": <string>`.
fn extract_version(
    value: &spanned::Value,
    line_index: &LineIndex,
) -> Option<(String, Span)> {
    if let Some(s) = value.as_str_spanned() {
        let span = string_inner_to_span(line_index, s.start(), s.end())?;
        return Some((s.get_ref().clone(), span));
    }
    if let Some(obj) = value.as_object() {
        let version_value = obj.get("version")?;
        let version_str = version_value.as_str_spanned()?;
        let span = string_inner_to_span(line_index, version_str.start(), version_str.end())?;
        return Some((version_str.get_ref().clone(), span));
    }
    None
}
```

> Note: the helper names `as_str_spanned` and `as_object` come from `json-spanned-value`'s `spanned::Value` API. If they differ in 0.2.2, adapt to whatever the crate exposes for `Spanned<&str>` and `Spanned<Object>` access. The intent is to read the inner span without losing the outer wrapper's range.

- [ ] **If the crate's actual API uses different method names** (verify by skimming `~/.cargo/registry/src/index.crates.io-*/json-spanned-value-0.2.2/src/value.rs`), adjust the calls but keep the same logic.

### Step 2.3: Run all `npm.rs` tests

- [ ] **Run:**

```bash
cargo test --package dependi-lsp --lib parsers::npm 2>&1 | tail -30
```

Expected: every test (existing + new) passes.

### Step 2.4: Lint, fmt, commit

- [ ] **Run:**

```bash
cargo clippy --package dependi-lsp --all-targets -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Commit:**

```bash
git add dependi-lsp/src/parsers/npm.rs
git commit -m "$(cat <<'EOF'
perf(npm): use span-aware JSON parser for position tracking (#236)

Replace the O(num_deps × document_length) string-scan in NpmParser with
spans produced by json-spanned-value, via the new parsers::json_spans
helpers. Also fixes a latent bug where the same dependency name appearing
in two sections (e.g. dependencies and devDependencies) could match the
wrong line. New tests cover same-name duplicates, substring false matches,
whitespace variations, and a 1000-dep smoke benchmark.
EOF
)"
```

---

## Task 3: Refactor `php.rs` to use spans

**Depends on:** Task 1 (independent of Task 2).

**Files:**
- Rewrite: `dependi-lsp/src/parsers/php.rs`

### Step 3.1: Add the new failing edge-case tests

- [ ] **Append to the `mod tests` block in `dependi-lsp/src/parsers/php.rs`:**

```rust
    #[test]
    fn test_same_name_in_require_and_require_dev() {
        let parser = PhpParser::new();
        let content = r#"{
  "require": {
    "vendor/foo": "1.0.0"
  },
  "require-dev": {
    "vendor/foo": "2.0.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let prod = deps.iter().find(|d| !d.dev).unwrap();
        let dev = deps.iter().find(|d| d.dev).unwrap();
        assert_eq!(prod.version, "1.0.0");
        assert_eq!(dev.version, "2.0.0");
        assert_ne!(prod.name_span.line, dev.name_span.line);
    }

    #[test]
    fn test_substring_false_match_in_value() {
        let parser = PhpParser::new();
        let content = r#"{
  "description": "contains \"vendor/fake\": \"99.0\" inside a string",
  "require": {
    "vendor/real": "^1.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "vendor/real");
    }

    #[test]
    fn test_skip_php_and_ext_when_duplicates_present() {
        let parser = PhpParser::new();
        let content = r#"{
  "require": {
    "php": ">=8.1",
    "ext-json": "*",
    "ext-mbstring": "*",
    "vendor/lib": "^1.0"
  },
  "require-dev": {
    "php": ">=8.1",
    "ext-json": "*",
    "vendor/dev": "^1.0"
  }
}"#;
        let deps = parser.parse(content);
        // Only the two real packages, not php or ext-* in either section.
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "vendor/lib" && !d.dev));
        assert!(deps.iter().any(|d| d.name == "vendor/dev" && d.dev));
    }
```

- [ ] **Run the tests, expect at least the duplicate-name test to fail:**

```bash
cargo test --package dependi-lsp --lib parsers::php 2>&1 | tail -30
```

### Step 3.2: Rewrite `php.rs` to use spans

- [ ] **Replace the entire module body (keep the test module at the bottom intact, including the new tests from 3.1):**

```rust
//! Parser for PHP Composer files (composer.json)
//!
//! Uses `json-spanned-value` for span tracking; the PHP-specific filter rules
//! (skip `php` and `ext-*` keys) are applied before any work is done with the
//! span info.

use json_spanned_value as jsv;
use json_spanned_value::spanned;

use super::json_spans::{LineIndex, inner_string_span, span_to_span};
use super::{Dependency, Parser, Span};

/// Parser for PHP composer.json dependency files.
#[derive(Debug, Default)]
pub struct PhpParser;

impl PhpParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for PhpParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let Ok(root) = jsv::from_str::<spanned::Object>(content) else {
            return Vec::new();
        };

        let line_index = LineIndex::new(content);
        let mut dependencies = Vec::with_capacity(32);

        parse_section(&root, "require", false, &line_index, &mut dependencies);
        parse_section(&root, "require-dev", true, &line_index, &mut dependencies);

        dependencies
    }
}

fn parse_section(
    root: &spanned::Object,
    section_name: &str,
    dev: bool,
    line_index: &LineIndex,
    dependencies: &mut Vec<Dependency>,
) {
    let Some(section_value) = root.get(section_name) else {
        return;
    };
    let Some(section_obj) = section_value.as_object() else {
        return;
    };

    for (name_spanned, value_spanned) in section_obj {
        let name = name_spanned.get_ref();
        if name == "php" || name.starts_with("ext-") {
            continue;
        }

        let Some(version_spanned) = value_spanned.as_str_spanned() else {
            continue;
        };

        let Some(name_span) =
            string_inner_to_span(line_index, name_spanned.start(), name_spanned.end())
        else {
            continue;
        };
        let Some(version_span) =
            string_inner_to_span(line_index, version_spanned.start(), version_spanned.end())
        else {
            continue;
        };
        if name_span.line != version_span.line {
            continue;
        }

        dependencies.push(Dependency {
            name: name.clone(),
            version: version_spanned.get_ref().clone(),
            name_span,
            version_span,
            dev,
            optional: false,
            registry: None,
            resolved_version: None,
        });
    }
}

fn string_inner_to_span(line_index: &LineIndex, start: usize, end: usize) -> Option<Span> {
    let (inner_start, inner_end) = inner_string_span(start, end);
    span_to_span(line_index, inner_start, inner_end)
}
```

> Same caveat as Task 2 about adjusting `as_object` / `as_str_spanned` to whatever names `json-spanned-value` 0.2.2 actually exposes — verify against the crate source if compilation fails.

### Step 3.3: Run all `php.rs` tests

- [ ] **Run:**

```bash
cargo test --package dependi-lsp --lib parsers::php 2>&1 | tail -30
```

Expected: every test passes.

### Step 3.4: Lint, fmt, commit

- [ ] **Run:**

```bash
cargo clippy --package dependi-lsp --all-targets -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Commit:**

```bash
git add dependi-lsp/src/parsers/php.rs
git commit -m "$(cat <<'EOF'
perf(php): use span-aware JSON parser for position tracking (#236)

Symmetric refactor of the PHP composer.json parser: drop the manual
string scan, read name/version spans from json-spanned-value via the
shared parsers::json_spans helpers. Fixes the same-name-in-two-sections
bug and adds tests for it plus substring false-match scenarios.
EOF
)"
```

---

## Task 4: Run benchmarks and update CHANGELOG

**Depends on:** Tasks 2 + 3.

**Files:**
- Modify: `CHANGELOG.md`

### Step 4.1: Run the package_json + composer_json benches before the refactor

(For the agent: if the refactor is already merged, skip this and use the issue's stated 30–50 % target as the success bar. Otherwise, capture baseline numbers from `main` first.)

### Step 4.2: Run benches on the refactor branch

- [ ] **Run:**

```bash
cargo bench --package dependi-lsp --bench benchmarks -- package_json composer_json 2>&1 | tee /tmp/bench-after.txt
```

- [ ] **Confirm parse times for `package_json` and `composer_json` improve at large `dep_count` (target ≥30 %).**

### Step 4.3: Update `CHANGELOG.md`

- [ ] **Open `CHANGELOG.md`. Under `## [Unreleased]`, add (creating subsections if needed):**

```markdown
### Performance

- npm and PHP parsers now use a span-aware JSON parser
  (`json-spanned-value`) instead of an O(num_deps × document_length)
  string scan. Parse time on large `package.json` / `composer.json`
  files improves by roughly 30–50 %. (#236)

### Fixed

- npm and PHP parsers no longer mismatch dependencies whose name appears
  in more than one section (e.g. both `dependencies` and
  `devDependencies`, or `require` and `require-dev`). (#236)

### Changed

- New shared module `parsers::json_spans` (`LineIndex`, span helpers)
  used by the npm and PHP parsers.
```

### Step 4.4: Commit

- [ ] **Run:**

```bash
git add CHANGELOG.md
git commit -m "$(cat <<'EOF'
docs(changelog): note npm/php JSON span parser refactor (#236)
EOF
)"
```

---

## Final verification

- [ ] **Full lint + fmt + tests green:**

```bash
cargo clippy --package dependi-lsp --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --package dependi-lsp --lib
cargo test --package dependi-lsp --test integration_test
```

All four must succeed.
