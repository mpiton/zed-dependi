# Design — Position-tracking JSON parser for npm and PHP parsers

**Issue**: [#236](https://github.com/mpiton/zed-dependi/issues/236)
**Date**: 2026-04-27
**Branch**: `feat/issue-236-npm-php-json-spans` (à créer)
**Status**: Draft

## Context

`dependi-lsp/src/parsers/npm.rs` and `dependi-lsp/src/parsers/php.rs` currently parse
their JSON manifest files (`package.json`, `composer.json`) using `serde_json::Value`,
then locate each dependency's line/column position by **scanning the original document
string** with `content.find()`. This produces O(num_deps × document_length) behaviour
on large files and contains a latent correctness bug: if the same dependency name
appears in two sections (e.g. both `dependencies` and `devDependencies`), the string
search may match the wrong occurrence.

Issue #236 requests a position-tracking JSON parser to remove the manual search.
The issue mentions `json_comments::from_str_with_span`, which does not exist
(`json_comments` only strips comments). The actual span-tracking crate selected
during brainstorming is `json-spanned-value` 0.2.2.

## Goals

- Replace `compute_line_offsets` + `find_dependency_position` in `npm.rs` and `php.rs`
  with span information produced by the parser itself.
- Eliminate the duplicate-name false-match bug.
- Achieve ≥30 % parse time improvement on large `package.json` / `composer.json`
  files (per issue target, validated via existing Criterion benchmarks).
- Factor common helpers (`LineIndex`, span → `Span` conversion) into a shared
  module `parsers/json_spans.rs`.

## Non-goals

- Replacing `serde_json` everywhere in the codebase (only npm + php JSON parsers).
- Refactoring lock-file JSON parsers (`npm_lock.rs`, `composer_lock.rs`,
  `packages_lock_json.rs`) — those do not currently use position tracking.
- Switching column units from byte offsets to UTF-16 code units (LSP spec). Current
  parsers already emit byte columns; this PR preserves that behaviour.

## Architecture

```
dependi-lsp/src/parsers/
├── json_spans.rs       (NEW — shared helpers)
├── npm.rs              (refactored)
└── php.rs              (refactored)
```

`json_spans.rs` exports:

- `struct LineIndex` — pre-computed `Vec<usize>` of byte offsets at each line start.
  Constructed once per parse, queried O(log n) per position via binary search.
- `fn span_to_span(line_index: &LineIndex, byte_start: usize, byte_end: usize) -> Span`
  — converts a byte range to the project's `Span { line, line_start, line_end }`.
  Returns `None` if start/end straddle multiple lines, so callers can preserve
  the existing same-line invariant.
- `fn string_inner_to_span(line_index: &LineIndex, start: usize, end: usize) -> Option<Span>`
  — strips the surrounding `"…"` from a JSON string's outer (quote-inclusive)
  byte range and converts the inner content to a `Span`. Returns `None` if the
  span is shorter than the two surrounding quotes or straddles a line boundary.

`npm.rs` and `php.rs` use `json_spanned_value::from_str` to deserialize into
strongly-typed structures using `Spanned<…>` wrappers, then convert each pair
of (name, version) spans into `Dependency` entries.

## Data flow (npm)

1. `json_spanned_value::from_str::<spanned::Object>(content)` → root object (or
   `Vec::new()` on error).
2. `LineIndex::new(content)` (single O(n) pass).
3. For each known section name (`dependencies`, `devDependencies`,
   `peerDependencies`, `optionalDependencies`):
   - `root.get(section)` → `Option<&Spanned<Value>>`.
   - Coerce to `spanned::Object`.
   - For each (`Spanned<Str>` key, `Spanned<Value>` value):
     - Skip the entry if the value is neither a `Spanned<String>` nor a
       `Spanned<Object>` containing a `"version": Spanned<String>` field.
     - Compute `name_span` from the inner span of the key, `version_span` from the
       inner span of the version string.
     - Verify both spans land on the same line (drop the entry otherwise — preserves
       the existing invariant).
     - Push `Dependency { dev, optional, … }`.

PHP flow is symmetric, with sections `require` / `require-dev`, plus filters for
`name == "php"` and `name.starts_with("ext-")`.

## Error handling

| Case | Behaviour |
|---|---|
| `from_str` returns `Err` | Return `Vec::new()` (matches current `serde_json` behaviour). |
| Missing section | Skip silently (current behaviour). |
| Section not an object | Skip silently (current behaviour). |
| Version not a string and not an object with `"version"` | Skip the entry. |
| Multi-line value (key on one line, value on next) | Skip the entry — preserves existing same-line invariant. |
| Multi-line object value with single-line `"version"` field | Use inner string span; OK. |
| UTF-8 multi-byte content | Columns are byte offsets, same as today. |

## Testing strategy (TDD)

Each unit lands red → green → refactor before moving to the next.

### `json_spans.rs` (new tests)

- `LineIndex::new("")` → `position(0) == (0, 0)`.
- Position of first byte after a `\n` is `(line + 1, 0)`.
- Last byte of a non-newline-terminated line returns the correct line.
- Multi-byte UTF-8 character positioning (4-byte emoji): byte column matches.

### `npm.rs` — existing tests pass unchanged

All 10 tests in `dependi-lsp/src/parsers/npm.rs#tests` continue to pass.

### `npm.rs` — new tests (cover bug fixes)

- `test_same_name_in_two_sections`: package `foo` appears in both `dependencies`
  and `devDependencies` with different versions. Expect two `Dependency` entries
  with distinct `dev` flags and spans on different lines (current bug: may
  match wrong line).
- `test_substring_false_match`: a value string contains the literal text
  `"react": "1.0.0"` (e.g. inside a description). Parser must not treat it as
  a dependency entry.
- `test_whitespace_variations`: tabs and multiple spaces between key, colon,
  value — spans still correct.
- `test_trailing_newlines_and_bom`: BOM-prefixed JSON parses; trailing newlines
  do not shift spans.
- `test_large_file_smoke`: 1 000 dependencies parse in well under 1 s
  (smoke check, not a strict perf assertion — the Criterion bench provides that).

### `php.rs` — symmetric new tests

- `test_same_name_in_require_and_require_dev`.
- `test_substring_false_match`.
- `test_skip_php_and_ext_when_duplicates_present`: `php` and `ext-json` keys
  appear with versions that look like real deps; they remain skipped.

### Benchmarks

`dependi-lsp/benches/benchmarks.rs` already contains `package_json` and
`composer_json` Criterion benches. Re-run before/after to confirm ≥30 %
improvement on `dep_count = 100`.

## Trade-offs

- **New dependency**: `json-spanned-value` (Apache-2.0 / MIT, ~10 transitive deps,
  built on serde_json which is already present). Maintenance risk is low because
  the crate is a thin layer over serde_json.
- **Span granularity**: byte offsets, not UTF-16. Same as today.
- **Multi-line values dropped silently**: preserves existing invariant; if this
  becomes a real-world issue we can revisit (out of scope for #236).

## Implementation plan (handed off to writing-plans)

Phases (TDD per phase):

1. Add `json-spanned-value` to `dependi-lsp/Cargo.toml` (deps).
2. Create `parsers/json_spans.rs` with `LineIndex` + helpers, full unit tests.
3. Wire `pub mod json_spans;` in `parsers/mod.rs`.
4. Refactor `npm.rs` to use `json_spanned_value` + `json_spans`. New tests first.
5. Refactor `php.rs` symmetrically. New tests first.
6. Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`.
7. Run `cargo bench --bench benchmarks -- package_json composer_json` before/after,
   record numbers in PR description.
8. Update `CHANGELOG.md` under `[Unreleased] / Changed` and `Performance`.

## Affected files

- `dependi-lsp/Cargo.toml` (+1 dep)
- `dependi-lsp/src/parsers/mod.rs` (+1 mod line)
- `dependi-lsp/src/parsers/json_spans.rs` (new file ~80 lines + tests)
- `dependi-lsp/src/parsers/npm.rs` (rewrite parser + new tests, net ~ −150 lines)
- `dependi-lsp/src/parsers/php.rs` (rewrite parser + new tests, net ~ −100 lines)
- `CHANGELOG.md` (entry under `[Unreleased]`)
