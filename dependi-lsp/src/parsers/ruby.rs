//! Parser for Ruby Gemfile files
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
        let mut dependencies = Vec::new();
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
    if !trimmed.starts_with("gem ") && !trimmed.starts_with("gem(") {
        return None;
    }

    // Extract the part after 'gem'
    let after_gem = if trimmed.starts_with("gem(") {
        trimmed.strip_prefix("gem(")?.trim_end_matches(')')
    } else {
        trimmed.strip_prefix("gem ")?
    };

    // Split by comma to get arguments
    let args: Vec<&str> = split_gem_args(after_gem);

    if args.is_empty() {
        return None;
    }

    // First argument is the gem name (quoted)
    let name = args[0].trim().trim_matches(|c| c == '\'' || c == '"');

    // Second argument (if present) is the version constraint
    let version = if args.len() > 1 {
        let version_arg = args[1].trim();
        // Skip if it's a hash option (like require: false, git: ...)
        if version_arg.contains(':')
            && !version_arg.starts_with('\'')
            && !version_arg.starts_with('"')
        {
            return None; // No version, has options like git: or path:
        }
        version_arg
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string()
    } else {
        return None; // No version specified
    };

    // Skip if version looks like a hash key (path, git, etc.)
    if version.is_empty() || version.contains(':') {
        return None;
    }

    // Calculate positions in the original line
    let name_start = find_string_position(line, name)? as u32;
    let name_end = name_start + name.len() as u32;
    let version_start = find_string_position(line, &version)? as u32;
    let version_end = version_start + version.len() as u32;

    Some(Dependency {
        name: name.to_string(),
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

/// Split gem arguments respecting quotes
fn split_gem_args(s: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for (i, c) in s.char_indices() {
        match c {
            '\'' | '"' if !in_quotes => {
                in_quotes = true;
                quote_char = c;
            }
            c if c == quote_char && in_quotes => {
                in_quotes = false;
            }
            ',' if !in_quotes => {
                let arg = s[start..i].trim();
                if !arg.is_empty() {
                    args.push(arg);
                }
                start = i + 1;
            }
            _ => {}
        }
    }

    // Don't forget the last argument
    let last_arg = s[start..].trim();
    if !last_arg.is_empty() {
        // Stop at hash options
        if let Some(hash_start) = last_arg.find([':', '{']) {
            let before_hash = last_arg[..hash_start].trim();
            if !before_hash.is_empty()
                && (before_hash.starts_with('\'') || before_hash.starts_with('"'))
            {
                args.push(before_hash);
            }
        } else {
            args.push(last_arg);
        }
    }

    args
}

/// Find the position of a string in a line (accounting for quotes)
fn find_string_position(line: &str, needle: &str) -> Option<usize> {
    // Look for the string within quotes
    for quote in &['\'', '"'] {
        let quoted = format!("{}{}{}", quote, needle, quote);
        if let Some(pos) = line.find(&quoted) {
            return Some(pos + 1); // Skip the opening quote
        }
    }
    // Fallback to direct search
    line.find(needle)
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
}
