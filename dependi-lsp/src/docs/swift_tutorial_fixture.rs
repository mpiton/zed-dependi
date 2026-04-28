//! Doctest fixtures backing `docs/adding-a-language.md`.
//!
//! Every example shown in the tutorial that is fenced ```rust (without
//! `ignore`) is reproduced as a doctest in this module. Failures here
//! mean the tutorial has drifted from reality.
//!
//! # Example 1 — Constructing a [`Span`]
//!
//! A [`Span`] covers the *inner* bytes of the token (no quotes), measured
//! relative to the start of the line.
//!
//! ```
//! use dependi_lsp::parsers::Span;
//!
//! // Imagine the source line: `    .package(url: "https://example.com/foo", from: "1.0.0"),`
//! //                                                                    ^^^^^
//! //                                                                    bytes 60..65
//! let version_span = Span {
//!     line: 4,
//!     line_start: 60,
//!     line_end: 65,
//! };
//! assert_eq!(version_span.line_end - version_span.line_start, 5);
//! ```
//!
//! [`Span`]: dependi_lsp::parsers::Span
//!
//! # Placeholder doctests (replaced in later tasks)
//!
//! # Example 2 — Constructing a [`Dependency`]
//!
//! Each call to [`Parser::parse`] produces zero or more [`Dependency`]
//! values. The two `Span` fields anchor LSP quick-fix edits.
//!
//! ```
//! use dependi_lsp::parsers::{Dependency, Span};
//!
//! let dep = Dependency {
//!     name: "swift-argument-parser".to_string(),
//!     version: "1.3.0".to_string(),
//!     name_span: Span { line: 4, line_start: 32, line_end: 53 },
//!     version_span: Span { line: 4, line_start: 60, line_end: 65 },
//!     dev: false,
//!     optional: false,
//!     registry: None,
//!     resolved_version: None,
//! };
//! assert_eq!(dep.effective_version(), "1.3.0");
//! ```
//!
//! [`Dependency`]: dependi_lsp::parsers::Dependency
//! [`Parser::parse`]: dependi_lsp::parsers::Parser::parse
//! # Example 3 — Implementing the [`Parser`] trait
//!
//! A parser receives the manifest contents as `&str` and returns a
//! `Vec<Dependency>`. The implementation below uses naïve substring
//! matching — production parsers should handle escaping, comments, and
//! the full DSL grammar.
//!
//! ```
//! use dependi_lsp::parsers::{Dependency, Parser, Span};
//!
//! struct SwiftParser;
//!
//! impl Parser for SwiftParser {
//!     fn parse(&self, content: &str) -> Vec<Dependency> {
//!         let mut deps = Vec::new();
//!         for (line_idx, line) in content.lines().enumerate() {
//!             let trimmed = line.trim_start();
//!             if !trimmed.starts_with(".package(url:") {
//!                 continue;
//!             }
//!             let url_start = match line.find('"') {
//!                 Some(idx) => idx + 1,
//!                 None => continue,
//!             };
//!             let url_end = match line[url_start..].find('"') {
//!                 Some(idx) => url_start + idx,
//!                 None => continue,
//!             };
//!             let url = &line[url_start..url_end];
//!             let name = url.rsplit('/').next().unwrap_or(url).to_string();
//!             let version_marker = match line.rfind('"').and_then(|end| {
//!                 line[..end].rfind('"').map(|start| (start + 1, end))
//!             }) {
//!                 Some(pair) if pair.0 > url_end => pair,
//!                 _ => continue,
//!             };
//!             let version = line[version_marker.0..version_marker.1].to_string();
//!             deps.push(Dependency {
//!                 name,
//!                 version,
//!                 name_span: Span {
//!                     line: line_idx as u32,
//!                     line_start: url_start as u32,
//!                     line_end: url_end as u32,
//!                 },
//!                 version_span: Span {
//!                     line: line_idx as u32,
//!                     line_start: version_marker.0 as u32,
//!                     line_end: version_marker.1 as u32,
//!                 },
//!                 dev: false,
//!                 optional: false,
//!                 registry: None,
//!                 resolved_version: None,
//!             });
//!         }
//!         deps
//!     }
//! }
//!
//! let manifest = r#"
//! let package = Package(
//!     name: "MyApp",
//!     dependencies: [
//!         .package(url: "https://github.com/apple/swift-argument-parser", from: "1.3.0"),
//!     ]
//! )
//! "#;
//! let parser = SwiftParser;
//! let deps = parser.parse(manifest);
//! assert_eq!(deps.len(), 1);
//! assert_eq!(deps[0].name, "swift-argument-parser");
//! assert_eq!(deps[0].version, "1.3.0");
//! ```
//!
//! [`Parser`]: dependi_lsp::parsers::Parser
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 5");
//! ```
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 6");
//! ```
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 7");
//! ```
