---
title: Adding a New Language
layout: default
nav_order: 10
description: "Step-by-step guide for adding support for a new package manager / ecosystem to Dependi"
---

# Adding a New Language
{: .no_toc }

Step-by-step guide to adding a new language/ecosystem to Dependi. Worked example: Swift Package Manager.
{: .fs-6 .fw-300 }

<!--
  Every fenced ```rust block in this file (without `ignore`) is mirrored as
  a doctest in `dependi-lsp/src/docs/swift_tutorial_fixture.rs`. Edits to
  the snippets MUST be reflected there or the doctests drift.
-->

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

## 1. Introduction

This guide walks you through adding support for a new language or package manager to Dependi. By the end, your fork will detect the manifest file, parse its dependencies, fetch versions from the upstream registry, surface vulnerabilities via OSV.dev, and offer the same inlay hints, diagnostics, and code actions every other supported ecosystem gets.

The worked example throughout is **Swift Package Manager** (`Package.swift`). At the time of writing, SwiftPM is not yet supported, which makes it a good candidate: you can follow the tutorial end-to-end and ship a real PR. If you target a different ecosystem, use the example as a template — the wire-up steps are identical.

### What you need before you start

- **Rust 1.94 or newer** (this repository is on edition 2024).
- **Git, Cargo, and the `wasm32-wasip1` target**: `rustup target add wasm32-wasip1`.
- **Familiarity with `async`/`await`**. Registry clients are async; parsers are synchronous.
- **A sample manifest from your target ecosystem** to drive your first test.
- **The OSV.dev ecosystem name**, if your registry is in OSV's coverage list. Look it up at <https://ossf.github.io/osv-schema/#defined-ecosystems> before starting Step 4. For SwiftPM the value the tutorial uses is `"SwiftURL"`; verify against the schema in case it has changed.

### What you'll touch

Five files (six if your ecosystem has lock files):

1. `dependi-lsp/src/file_types.rs` — file detection, ecosystem mapping, cache key.
2. `dependi-lsp/src/parsers/<your-lang>.rs` (new) plus `parsers/mod.rs` declaration.
3. `dependi-lsp/src/registries/<your-lang>.rs` (new) plus `registries/mod.rs` declaration.
4. `dependi-lsp/src/backend.rs` — `ProcessingContext` field, parser dispatch, registry dispatch.
5. `dependi-lsp/src/vulnerabilities/mod.rs` — `Ecosystem` variant + OSV string.
6. (Optional) `dependi-lsp/src/parsers/lockfile_resolver.rs` if your ecosystem has lock files.

The "Reference checklist" at the bottom of this page enumerates every individual edit so you can use it as a final review before opening your PR.

## 2. The big picture

_TBD — Task 11._

## 3. Step 1 — Define the file type

_TBD — Task 12._

## 4. Step 2 — Write the parser

_TBD — Task 13._

## 5. Step 3 — Write the registry client

_TBD — Task 14._

## 6. Step 4 — Wire into the backend

_TBD — Task 15._

## 7. Step 5 — (Optional) Lockfile resolver

_TBD — Task 16._

## 8. Step 6 — Update docs and CHANGELOG

_TBD — Task 17._

## 9. Verifying your work

_TBD — Task 18._

## 10. Reference checklist

_TBD — Task 19._

## 11. Common pitfalls

_TBD — Task 20._
