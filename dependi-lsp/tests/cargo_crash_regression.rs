//! Regression tests for malformed `Cargo.toml` inputs that previously
//! crashed [`CargoParser`].

use dependi_lsp::parsers::Parser;
use dependi_lsp::parsers::cargo::CargoParser;

/// Fuzz crash `56ee2249d56fe0b2e7564f6d7425e3104762e094`: an unterminated
/// basic-string value extends across multiple lines (the bytes between `}`
/// and `d-e_f"` are NUL padding produced by libFuzzer). The resulting
/// `Str` token spans several `line_ranges`, which used to make
/// `find_range_span` perform `needle.start() - line_range.start()` with
/// `needle.start() < line_range.start()`, triggering a `TextSize`
/// subtraction overflow.
#[test]
fn malformed_multiline_string_does_not_panic() {
    let bytes = include_bytes!("fixtures/cargo_text_size_underflow.bin");
    let content = std::str::from_utf8(bytes).expect("fixture is valid UTF-8");
    let _ = CargoParser::new().parse(content);
}
