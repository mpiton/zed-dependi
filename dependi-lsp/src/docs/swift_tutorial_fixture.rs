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
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 4");
//! ```
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 5");
//! ```
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 6");
//! ```
//! ```compile_fail
//! compile_error!("fixture not yet populated — Task 7");
//! ```
