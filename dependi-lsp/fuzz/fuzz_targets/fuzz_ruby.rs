#![no_main]

use dependi_lsp::parsers::ruby::RubyParser;
use dependi_lsp::parsers::Parser;
use libfuzzer_sys::fuzz_target;
use std::panic::AssertUnwindSafe;

fuzz_target!(|data: &[u8]| {
    if let Ok(content) = std::str::from_utf8(data) {
        let parser = RubyParser::new();

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| parser.parse(content)));

        if let Ok(deps) = result {
            let lines: Vec<&str> = content.lines().collect();

            for dep in &deps {
                assert!(
                    (dep.line as usize) < lines.len(),
                    "dep.line out of range"
                );

                let line = lines[dep.line as usize];
                let line_len = line.len() as u32;

                assert!(
                    dep.name_start <= dep.name_end,
                    "name_start must be <= name_end"
                );
                assert!(
                    dep.name_start <= line_len,
                    "name_start must be within line bounds"
                );
                assert!(
                    dep.name_end <= line_len,
                    "name_end must be within line bounds"
                );
                assert!(
                    dep.version_start <= dep.version_end,
                    "version_start must be <= version_end"
                );
                assert!(
                    dep.version_start <= line_len,
                    "version_start must be within line bounds"
                );
                assert!(
                    dep.version_end <= line_len,
                    "version_end must be within line bounds"
                );
            }
        }
    }
});
