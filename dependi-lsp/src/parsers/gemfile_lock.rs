//! Parser for Ruby lockfiles (Gemfile.lock) — resolves exact locked versions for Ruby gems.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Platform architecture keywords used to detect platform suffixes in gem versions.
const PLATFORM_KEYWORDS: &[&str] = &[
    "x86_64",
    "x86",
    "arm64",
    "aarch64",
    "darwin",
    "linux",
    "mingw",
    "mswin",
    "java",
    "jruby",
    "universal",
];

/// Strip platform suffix from a gem version string.
///
/// RubyGems platform-specific gems append a platform suffix separated by `-`,
/// e.g. `1.15.4-x86_64-linux`. The actual version is the part before the first `-`
/// that is followed by a platform keyword.
///
/// Pre-release Ruby versions use dots, not hyphens (e.g. `1.0.0.pre.1`), so they
/// are returned unchanged.
fn strip_platform_suffix(version: &str) -> &str {
    if let Some(dash_pos) = version.find('-') {
        let after_dash = &version[dash_pos + 1..];
        // Check if the part after the first dash looks like a platform suffix
        let is_platform = PLATFORM_KEYWORDS.iter().any(|kw| {
            after_dash.starts_with(kw)
                && after_dash[kw.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| c == '-' || c.is_ascii_digit())
        });
        if is_platform {
            return &version[..dash_pos];
        }
    }
    version
}

/// Normalize a Ruby gem name to lowercase for case-insensitive matching.
pub fn normalize_gem_name(name: &str) -> String {
    name.to_lowercase()
}

/// Parse a Gemfile.lock file and return a map of gem name → resolved version.
///
/// Only `GEM` (registry) sections are resolved. `PATH` and `GIT` sourced
/// gems are intentionally excluded — they are also skipped by the Gemfile manifest
/// parser (`ruby.rs`) and do not need version resolution against a registry.
///
/// Extracts versions from the GEM specs section where each gem appears as:
///   `    gem_name (VERSION)`
/// with exactly 4 spaces of indentation.
///
/// Gem names are stored in lowercase for case-insensitive matching.
/// Platform suffixes (e.g., `-x86_64-linux`) are stripped from versions.
pub fn parse_gemfile_lock(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    #[derive(PartialEq)]
    enum State {
        Searching,
        InGem,
        InSpecs,
    }

    let mut state = State::Searching;

    for line in content.lines() {
        match state {
            State::Searching => {
                if line == "GEM" {
                    state = State::InGem;
                }
            }
            State::InGem => {
                if line == "  specs:" {
                    state = State::InSpecs;
                } else if !line.starts_with(' ') && !line.is_empty() {
                    // New top-level section, not in GEM anymore
                    state = State::Searching;
                    // Re-check if this is another GEM section
                    if line == "GEM" {
                        state = State::InGem;
                    }
                }
            }
            State::InSpecs => {
                // A new top-level section (no leading spaces) ends the specs block
                if !line.is_empty() && !line.starts_with(' ') {
                    state = State::Searching;
                    if line == "GEM" {
                        state = State::InGem;
                    }
                    continue;
                }

                // Count leading spaces to determine indentation level
                let leading_spaces = line.len() - line.trim_start().len();

                // Exactly 4 spaces = direct gem entry
                if leading_spaces == 4 {
                    let trimmed = line.trim();
                    if let Some((name, rest)) = trimmed.split_once(' ') {
                        // rest should be like "(1.2.3)" or "(1.2.3-x86_64-linux)"
                        if rest.starts_with('(') && rest.ends_with(')') {
                            let raw_version = &rest[1..rest.len() - 1];
                            let version = strip_platform_suffix(raw_version).to_string();
                            let key = normalize_gem_name(name);
                            map.entry(key).or_insert(version);
                        }
                    }
                }
                // Lines with 6+ spaces are sub-dependencies — skip them
            }
        }
    }

    map
}

/// Find the Gemfile.lock file co-located with a Gemfile.
///
/// Bundler always places Gemfile.lock in the same directory as Gemfile,
/// so we only check the immediate directory — no upward traversal needed.
pub async fn find_gemfile_lock(gemfile_path: &Path) -> Option<PathBuf> {
    let candidate = gemfile_path.parent()?.join("Gemfile.lock");
    if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    rails (7.0.3.1)
      actioncable (= 7.0.3.1)
    nokogiri (1.15.4)

PLATFORMS
  ruby

DEPENDENCIES
  rails (~> 7.0.3)

BUNDLED WITH
  2.3.14
";
        let map = parse_gemfile_lock(content);
        assert_eq!(map.get("rails").map(|s| s.as_str()), Some("7.0.3.1"));
        assert_eq!(map.get("nokogiri").map(|s| s.as_str()), Some("1.15.4"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_empty() {
        let map = parse_gemfile_lock("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_skips_sub_dependencies() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    actioncable (7.0.3.1)
      actionpack (= 7.0.3.1)
      activesupport (= 7.0.3.1)
    rails (7.0.3.1)
      actioncable (= 7.0.3.1)
";
        let map = parse_gemfile_lock(content);
        // Only direct gems (4-space indent) should be included
        assert!(map.contains_key("actioncable"));
        assert!(map.contains_key("rails"));
        // Sub-dependencies like "actionpack" should NOT appear as top-level entries
        assert!(!map.contains_key("actionpack"));
        assert!(!map.contains_key("activesupport"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_multiple_gem_remotes() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    rails (7.0.3.1)

GEM
  remote: https://my.private.registry/
  specs:
    my_gem (1.0.0)

PLATFORMS
  ruby
";
        let map = parse_gemfile_lock(content);
        assert_eq!(map.get("rails").map(|s| s.as_str()), Some("7.0.3.1"));
        assert_eq!(map.get("my_gem").map(|s| s.as_str()), Some("1.0.0"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_platform_gem() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    nokogiri (1.15.4-x86_64-linux)
    grpc (1.59.0-x86_64-linux)
";
        let map = parse_gemfile_lock(content);
        assert_eq!(map.get("nokogiri").map(|s| s.as_str()), Some("1.15.4"));
        assert_eq!(map.get("grpc").map(|s| s.as_str()), Some("1.59.0"));
    }

    #[test]
    fn test_parse_stops_at_platforms() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    rails (7.0.3.1)

PLATFORMS
  x86_64-linux
  ruby

DEPENDENCIES
  rails (~> 7.0.3)
";
        let map = parse_gemfile_lock(content);
        // Should not accidentally parse PLATFORMS section content as gems
        assert!(map.contains_key("rails"));
        assert!(!map.contains_key("x86_64-linux"));
        assert!(!map.contains_key("ruby"));
    }

    #[test]
    fn test_parse_case_insensitive_names() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    ActiveRecord (7.0.3.1)
    JSON (2.6.3)
";
        let map = parse_gemfile_lock(content);
        // Names stored in lowercase
        assert!(map.contains_key("activerecord"));
        assert!(map.contains_key("json"));
        assert!(!map.contains_key("ActiveRecord"));
        assert!(!map.contains_key("JSON"));
    }

    #[test]
    fn test_parse_prerelease_version() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    my_gem (1.0.0.pre.1)
    another_gem (2.0.0.beta.2)
";
        let map = parse_gemfile_lock(content);
        // Pre-release versions use dots, not hyphens — kept as-is
        assert_eq!(map.get("my_gem").map(|s| s.as_str()), Some("1.0.0.pre.1"));
        assert_eq!(
            map.get("another_gem").map(|s| s.as_str()),
            Some("2.0.0.beta.2")
        );
    }

    #[test]
    fn test_parse_malformed_lines() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    valid_gem (1.0.0)
    no_version_gem
    bad_parens (
    also_bad )
";
        let map = parse_gemfile_lock(content);
        // Only properly formatted lines should be parsed
        assert_eq!(map.get("valid_gem").map(|s| s.as_str()), Some("1.0.0"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_strip_platform_suffix_arm64() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    nokogiri (1.15.4-arm64-darwin)
";
        let map = parse_gemfile_lock(content);
        assert_eq!(map.get("nokogiri").map(|s| s.as_str()), Some("1.15.4"));
    }

    #[test]
    fn test_duplicate_gem_keeps_first() {
        let content = "\
GEM
  remote: https://rubygems.org/
  specs:
    rails (7.0.3.1)
    rails (6.0.0)
";
        let map = parse_gemfile_lock(content);
        assert_eq!(map.get("rails").map(|s| s.as_str()), Some("7.0.3.1"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_parse_ignores_path_and_git_sections() {
        let content = "\
PATH
  remote: ../my_engine
  specs:
    my_engine (0.1.0)
      rails (>= 7.0)

GIT
  remote: https://github.com/user/repo.git
  revision: abc123
  specs:
    some_gem (2.0.0)

GEM
  remote: https://rubygems.org/
  specs:
    rails (7.0.3.1)

PLATFORMS
  ruby
";
        let map = parse_gemfile_lock(content);
        assert_eq!(map.get("rails").map(|s| s.as_str()), Some("7.0.3.1"));
        assert!(!map.contains_key("my_engine"));
        assert!(!map.contains_key("some_gem"));
        assert_eq!(map.len(), 1);
    }
}
