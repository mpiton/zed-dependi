//! Parser for Ruby Gemfile files
//!
//! Optimized for performance with reduced allocations.
//!
//! Supports:
//! - Gemfile format (Bundler)
//! - gem declarations with version constraints
//! - group blocks for development dependencies

use super::{Dependency, Parser};

/// Parser for Ruby Gemfile dependency files
#[derive(Debug, Default)]
pub struct RubyParser;

impl RubyParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for RubyParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        // Pre-allocate with reasonable capacity
        let mut dependencies = Vec::with_capacity(32);
        let mut in_dev_group = false;
        let mut group_depth = 0;

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Skip comments and empty lines
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Track group blocks for dev dependencies
            if trimmed.starts_with("group") {
                let is_dev = trimmed.contains(":development")
                    || trimmed.contains(":test")
                    || trimmed.contains("'development'")
                    || trimmed.contains("\"development\"")
                    || trimmed.contains("'test'")
                    || trimmed.contains("\"test\"");

                if trimmed.contains("do") {
                    group_depth += 1;
                    if is_dev {
                        in_dev_group = true;
                    }
                }
                continue;
            }

            // Track end of blocks
            if trimmed == "end" {
                if group_depth > 0 {
                    group_depth -= 1;
                    if group_depth == 0 {
                        in_dev_group = false;
                    }
                }
                continue;
            }

            // Parse gem declarations
            if let Some(dep) = parse_gem_declaration(line, line_num, in_dev_group) {
                dependencies.push(dep);
            }
        }

        dependencies
    }
}

/// Parse a gem declaration from a line
fn parse_gem_declaration(line: &str, line_num: u32, dev: bool) -> Option<Dependency> {
    let trimmed = line.trim();

    // Must start with 'gem'
    let after_gem = if let Some(rest) = trimmed.strip_prefix("gem(") {
        // Use strip_suffix to remove at most one trailing ')'
        rest.strip_suffix(')').unwrap_or(rest)
    } else if let Some(rest) = trimmed.strip_prefix("gem ") {
        rest
    } else {
        return None;
    };

    // Parse the arguments
    let (name, version, name_start, name_end, version_start, version_end) =
        parse_gem_args(line, after_gem)?;

    Some(Dependency {
        name,
        version,
        line: line_num,
        name_start,
        name_end,
        version_start,
        version_end,
        dev,
        optional: false,
    })
}

/// Parse gem arguments and return (name, version, positions)
fn parse_gem_args(line: &str, args_str: &str) -> Option<(String, String, u32, u32, u32, u32)> {
    let bytes = args_str.as_bytes();
    let len = bytes.len();

    // Parse first argument (name)
    let (name, name_end_idx) = parse_quoted_string(bytes, 0)?;

    // Find comma after name
    let mut idx = name_end_idx;
    while idx < len && bytes[idx] != b',' {
        idx += 1;
    }
    if idx >= len {
        return None; // No version
    }
    idx += 1; // Skip comma

    // Skip whitespace
    while idx < len && (bytes[idx] == b' ' || bytes[idx] == b'\t') {
        idx += 1;
    }
    if idx >= len {
        return None;
    }

    // Check if this looks like a hash option (contains : but not quoted)
    let next_byte = bytes[idx];
    if next_byte != b'\'' && next_byte != b'"' {
        // Not a quoted string, likely a hash option like git:
        return None;
    }

    // Parse second argument (version)
    let (version, _) = parse_quoted_string(bytes, idx)?;

    // Skip if version looks like a hash key
    if version.is_empty() || version.contains(':') {
        return None;
    }

    // Find positions in the original line
    let (name_start, name_end) = find_quoted_position(line, &name)?;
    let (version_start, version_end) = find_quoted_position(line, &version)?;

    Some((
        name,
        version,
        name_start,
        name_end,
        version_start,
        version_end,
    ))
}

/// Parse a quoted string starting at index, return (string, end_index)
fn parse_quoted_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let len = bytes.len();
    let mut idx = start;

    // Skip whitespace
    while idx < len && (bytes[idx] == b' ' || bytes[idx] == b'\t') {
        idx += 1;
    }
    if idx >= len {
        return None;
    }

    let quote = bytes[idx];
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    idx += 1;

    let string_start = idx;
    while idx < len && bytes[idx] != quote {
        idx += 1;
    }
    if idx >= len {
        return None;
    }

    let s = std::str::from_utf8(&bytes[string_start..idx]).ok()?;
    Some((s.to_string(), idx + 1))
}

/// Find the position of a quoted string in a line
fn find_quoted_position(line: &str, needle: &str) -> Option<(u32, u32)> {
    // Look for the string within single quotes first (more common in Ruby)
    let single_quoted = format!("'{}'", needle);
    if let Some(pos) = line.find(&single_quoted) {
        let start = (pos + 1) as u32;
        return Some((start, start + needle.len() as u32));
    }

    // Try double quotes
    let double_quoted = format!("\"{}\"", needle);
    if let Some(pos) = line.find(&double_quoted) {
        let start = (pos + 1) as u32;
        return Some((start, start + needle.len() as u32));
    }

    // Fallback to direct search
    let pos = line.find(needle)? as u32;
    Some((pos, pos + needle.len() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_gem() {
        let parser = RubyParser::new();
        let content = r#"
source 'https://rubygems.org'

gem 'rails', '~> 7.0'
gem 'pg', '~> 1.4'
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "rails");
        assert_eq!(deps[0].version, "~> 7.0");
        assert!(!deps[0].dev);
        assert_eq!(deps[1].name, "pg");
        assert_eq!(deps[1].version, "~> 1.4");
    }

    #[test]
    fn test_gem_with_version_operators() {
        let parser = RubyParser::new();
        let content = r#"
gem 'devise', '>= 4.0'
gem 'rspec', '~> 3.0'
gem 'rails', '~> 7.0.0'
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "devise");
        assert_eq!(deps[0].version, ">= 4.0");
        assert_eq!(deps[1].name, "rspec");
        assert_eq!(deps[1].version, "~> 3.0");
        assert_eq!(deps[2].name, "rails");
        assert_eq!(deps[2].version, "~> 7.0.0");
    }

    #[test]
    fn test_dev_dependencies_in_group() {
        let parser = RubyParser::new();
        let content = r#"
source 'https://rubygems.org'

gem 'rails', '~> 7.0'

group :development, :test do
  gem 'rspec-rails', '~> 6.0'
  gem 'factory_bot_rails', '~> 6.2'
end

gem 'pg', '~> 1.4'
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 4);

        let rails = deps.iter().find(|d| d.name == "rails").unwrap();
        assert!(!rails.dev);

        let rspec = deps.iter().find(|d| d.name == "rspec-rails").unwrap();
        assert!(rspec.dev);

        let factory_bot = deps.iter().find(|d| d.name == "factory_bot_rails").unwrap();
        assert!(factory_bot.dev);

        let pg = deps.iter().find(|d| d.name == "pg").unwrap();
        assert!(!pg.dev);
    }

    #[test]
    fn test_skip_git_and_path_gems() {
        let parser = RubyParser::new();
        let content = r#"
gem 'rails', '~> 7.0'
gem 'my_gem', git: 'https://github.com/user/my_gem.git'
gem 'local_gem', path: '../local_gem'
gem 'pg', '~> 1.4'
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "rails");
        assert_eq!(deps[1].name, "pg");
    }

    #[test]
    fn test_skip_comments_and_empty_lines() {
        let parser = RubyParser::new();
        let content = r#"
# This is a comment
source 'https://rubygems.org'

# Another comment
gem 'rails', '~> 7.0'

# gem 'old_gem', '1.0'
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "rails");
    }

    #[test]
    fn test_gem_with_require_option() {
        let parser = RubyParser::new();
        let content = r#"
gem 'rails', '~> 7.0'
gem 'bootsnap', '~> 1.16', require: false
gem 'pg', '~> 1.4'
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "rails");
        assert_eq!(deps[1].name, "bootsnap");
        assert_eq!(deps[1].version, "~> 1.16");
        assert_eq!(deps[2].name, "pg");
    }

    #[test]
    fn test_double_quoted_gems() {
        let parser = RubyParser::new();
        let content = r#"
gem "rails", "~> 7.0"
gem "pg", "~> 1.4"
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "rails");
        assert_eq!(deps[0].version, "~> 7.0");
    }

    #[test]
    fn test_version_positions() {
        let parser = RubyParser::new();
        let content = "gem 'rails', '~> 7.0'\n";
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 1);
        let dep = &deps[0];

        // Verify positions are valid
        assert!(dep.name_start < dep.name_end);
        assert!(dep.version_start < dep.version_end);
        assert!(dep.name_end < dep.version_start);

        // Verify name position
        let name_slice = &content[dep.name_start as usize..dep.name_end as usize];
        assert_eq!(name_slice, "rails");

        // Verify version position
        let version_slice = &content[dep.version_start as usize..dep.version_end as usize];
        assert_eq!(version_slice, "~> 7.0");
    }

    #[test]
    fn test_exact_version() {
        let parser = RubyParser::new();
        let content = "gem 'nokogiri', '1.15.4'\n";
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "nokogiri");
        assert_eq!(deps[0].version, "1.15.4");
    }

    #[test]
    fn test_test_group() {
        let parser = RubyParser::new();
        let content = r#"
gem 'rails', '~> 7.0'

group :test do
  gem 'rspec', '~> 3.12'
end
"#;
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 2);

        let rspec = deps.iter().find(|d| d.name == "rspec").unwrap();
        assert!(rspec.dev);
    }

    #[test]
    fn test_empty_file() {
        let parser = RubyParser::new();
        let content = "";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 0);
    }

    #[test]
    fn test_parenthesized_gem() {
        let parser = RubyParser::new();
        let content = "gem('rails', '~> 7.0')";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "rails");
        assert_eq!(deps[0].version, "~> 7.0");
    }
}
