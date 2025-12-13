//! Parser for Go module files (go.mod)

use super::{Dependency, Parser};

/// Parser for Go go.mod dependency files
#[derive(Debug, Default)]
pub struct GoParser;

impl GoParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for GoParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let mut dependencies = Vec::new();
        let mut in_require_block = false;

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            // Check for require block start
            if trimmed == "require (" {
                in_require_block = true;
                continue;
            }

            // Check for block end
            if trimmed == ")" {
                in_require_block = false;
                continue;
            }

            // Parse single-line require: require github.com/pkg/errors v0.9.1
            if trimmed.starts_with("require ") && !trimmed.contains("(") {
                let rest = &trimmed[8..]; // Skip "require "
                if let Some(dep) = parse_require_line(line, rest, line_num) {
                    dependencies.push(dep);
                }
                continue;
            }

            // Parse lines inside require block
            if in_require_block && let Some(dep) = parse_require_line(line, trimmed, line_num) {
                dependencies.push(dep);
            }
        }

        dependencies
    }
}

/// Parse a require line (either standalone or inside a block)
/// Format: module/path v1.2.3 [// indirect]
fn parse_require_line(line: &str, content: &str, line_num: u32) -> Option<Dependency> {
    let trimmed = content.trim();

    // Skip empty lines, comments, and replace directives
    if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("replace") {
        return None;
    }

    // Remove inline comment (// indirect or other comments)
    let is_indirect = trimmed.contains("// indirect");
    let without_comment = if let Some(pos) = trimmed.find("//") {
        trimmed[..pos].trim()
    } else {
        trimmed
    };

    // Split into module path and version
    let parts: Vec<&str> = without_comment.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let module_path = parts[0];
    let version = parts[1];

    // Version must start with 'v'
    if !version.starts_with('v') {
        return None;
    }

    // Calculate positions in the original line
    let name_start = line.find(module_path)? as u32;
    let name_end = name_start + module_path.len() as u32;
    let version_start = line.find(version)? as u32;
    let version_end = version_start + version.len() as u32;

    Some(Dependency {
        name: module_path.to_string(),
        version: version.to_string(),
        line: line_num,
        name_start,
        name_end,
        version_start,
        version_end,
        dev: false,
        optional: is_indirect, // Mark indirect dependencies as optional
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_require() {
        let parser = GoParser::new();
        let content = r#"
module example.com/mymodule

go 1.21

require github.com/pkg/errors v0.9.1
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "github.com/pkg/errors");
        assert_eq!(deps[0].version, "v0.9.1");
        assert!(!deps[0].optional);
    }

    #[test]
    fn test_require_block() {
        let parser = GoParser::new();
        let content = r#"
module example.com/mymodule

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    golang.org/x/text v0.14.0
)
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let gin = deps.iter().find(|d| d.name.contains("gin")).unwrap();
        assert_eq!(gin.version, "v1.9.1");

        let text = deps.iter().find(|d| d.name.contains("text")).unwrap();
        assert_eq!(text.version, "v0.14.0");
    }

    #[test]
    fn test_indirect_dependency() {
        let parser = GoParser::new();
        let content = r#"
require (
    github.com/direct/dep v1.0.0
    github.com/indirect/dep v2.0.0 // indirect
)
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let direct = deps.iter().find(|d| d.name.contains("direct")).unwrap();
        assert!(!direct.optional);

        let indirect = deps.iter().find(|d| d.name.contains("indirect")).unwrap();
        assert!(indirect.optional);
    }

    #[test]
    fn test_multiple_require_blocks() {
        let parser = GoParser::new();
        let content = r#"
require (
    github.com/pkg/a v1.0.0
)

require (
    github.com/pkg/b v2.0.0
)

require github.com/pkg/c v3.0.0
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);
    }

    #[test]
    fn test_version_position() {
        let parser = GoParser::new();
        let content = "require github.com/pkg/errors v0.9.1";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);

        let dep = &deps[0];
        // "require " is 8 chars (0-7)
        // "github.com/pkg/errors" is 21 chars (8-28), so name_end = 29
        // " " is at position 29
        // "v0.9.1" is 6 chars (30-35), so version_end = 36
        assert_eq!(dep.name_start, 8);
        assert_eq!(dep.name_end, 29);
        assert_eq!(dep.version_start, 30);
        assert_eq!(dep.version_end, 36);
    }

    #[test]
    fn test_skip_replace_directives() {
        let parser = GoParser::new();
        let content = r#"
require github.com/old/pkg v1.0.0

replace github.com/old/pkg => github.com/new/pkg v2.0.0
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "github.com/old/pkg");
    }
}
