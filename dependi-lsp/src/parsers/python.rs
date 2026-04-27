//! Parser for Python dependency files (requirements.txt, constraints.txt, pyproject.toml, hatch.toml)

use super::{Dependency, Parser, Span};

/// Parser for Python dependency files
#[derive(Debug, Default)]
pub struct PythonParser;

impl PythonParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for PythonParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        // Detect file type based on content.
        // Only parse as TOML if it contains valid pyproject.toml section headers.
        // Use line-anchored detection to avoid false positives like "mypkg[project]==1.2".
        // is_pyproject_toml is checked first so that a pyproject.toml that also uses
        // [tool.hatch.envs.*] is routed through parse_pyproject_toml (which handles both).
        if is_pyproject_toml(content) {
            parse_pyproject_toml(content)
        } else if is_hatch_toml(content) {
            parse_hatch_toml(content)
        } else {
            parse_requirements_txt(content)
        }
    }
}

/// Check if content is a pyproject.toml file by looking for line-anchored section headers
fn is_pyproject_toml(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();

        // Match [project...] section headers (e.g., [project], [project.dependencies])
        // Also allow inline comments: [project] # comment
        if trimmed.starts_with("[project") && is_valid_section_header(trimmed, "[project") {
            return true;
        }

        // Match [tool.poetry...] section headers (e.g., [tool.poetry], [tool.poetry.dependencies])
        if trimmed.starts_with("[tool.poetry") && is_valid_section_header(trimmed, "[tool.poetry") {
            return true;
        }

        // Match [dependency-groups] section header (PEP 735)
        if trimmed.starts_with("[dependency-groups")
            && is_valid_section_header(trimmed, "[dependency-groups")
        {
            return true;
        }
        // Match [tool.hatch...] section headers (e.g., [tool.hatch.envs.test])
        if trimmed.starts_with("[tool.hatch") && is_valid_section_header(trimmed, "[tool.hatch") {
            return true;
        }
    }
    false
}

/// Detect a standalone hatch.toml file.
///
/// The project-level Hatch config is always stored in a file named `hatch.toml`;
/// the filename cannot be changed. `file_types.rs` therefore gates entry to this
/// code path via a filename check, making content-based false positives impossible
/// in practice. Detection requires a top-level `[envs.<NAME>]` section header
/// with a mandatory dot-separated env name (bare `[envs]` is not a valid hatch section).
fn is_hatch_toml(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        // Require "[envs." (dot included) so that a bare "[envs]" is never matched.
        if trimmed.starts_with("[envs.")
            && let Some(close) = trimmed.find(']')
        {
            let after = trimmed[close + 1..].trim_start();
            if after.is_empty() || after.starts_with('#') {
                return true;
            }
        }
    }
    false
}

/// Check if a line is a valid TOML section header starting with the given prefix
/// Requires: starts with prefix, followed by either ']' or '.' then more chars ending with ']'
/// Allows optional whitespace and comments after the closing ']'
fn is_valid_section_header(line: &str, prefix: &str) -> bool {
    let after_prefix = &line[prefix.len()..];

    // Find the closing bracket
    let Some(bracket_pos) = after_prefix.find(']') else {
        return false;
    };

    // Check what's between prefix and ']': must be empty or start with '.'
    let inner = &after_prefix[..bracket_pos];
    if !inner.is_empty() && !inner.starts_with('.') {
        return false;
    }

    // Check what's after ']': must be only whitespace or a comment
    let after_bracket = after_prefix[bracket_pos + 1..].trim_start();
    after_bracket.is_empty() || after_bracket.starts_with('#')
}

/// Parse requirements.txt / constraints.txt format
/// Format: package==1.0.0, package>=1.0.0, package~=1.0.0, etc.
fn parse_requirements_txt(content: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx as u32;
        let trimmed = line.trim();

        // Skip empty lines, comments, and special directives
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('-')  // -r, -e, -c, etc.
            || trimmed.starts_with("--")
        // --index-url, etc.
        {
            continue;
        }

        // Skip URL dependencies (package @ https://...)
        if trimmed.contains(" @ ") {
            continue;
        }

        if let Some(dep) = parse_requirement_line(line, line_num, false) {
            dependencies.push(dep);
        }
    }

    dependencies
}

/// Parse a single requirement line
fn parse_requirement_line(line: &str, line_num: u32, dev: bool) -> Option<Dependency> {
    let trimmed = line.trim();

    // Remove inline comments
    let without_comment = if let Some(pos) = trimmed.find('#') {
        &trimmed[..pos]
    } else {
        trimmed
    };
    let without_comment = without_comment.trim();

    if without_comment.is_empty() {
        return None;
    }

    // Extract package name (before version specifier or extras)
    // Operators: ==, >=, <=, !=, ~=, >, <, ===
    let operators = ["===", "==", ">=", "<=", "!=", "~=", ">", "<"];

    let mut name_end_pos = without_comment.len();
    let mut version_op_pos = None;
    let mut version_op_len = 0;

    for op in &operators {
        if let Some(pos) = without_comment.find(op)
            && pos < name_end_pos
        {
            name_end_pos = pos;
            version_op_pos = Some(pos);
            version_op_len = op.len();
        }
    }

    // Handle extras: package[extra1,extra2]>=1.0
    let name_part = &without_comment[..name_end_pos];
    let name = if let Some(bracket_pos) = name_part.find('[') {
        &name_part[..bracket_pos]
    } else {
        name_part
    };
    let name = name.trim();

    if name.is_empty() {
        return None;
    }

    // Extract version (including the operator, to align with Ruby/npm behavior)
    let version = {
        let op_pos = version_op_pos?;
        let operator = &without_comment[op_pos..op_pos + version_op_len];
        let version_part = &without_comment[op_pos + version_op_len..];
        // Handle comma-separated version constraints: >=1.0,<2.0
        let version_num = if let Some(comma_pos) = version_part.find(',') {
            &version_part[..comma_pos]
        } else {
            version_part
        };
        // Remove environment markers: ; python_version >= "3.8"
        let version_num = if let Some(semi_pos) = version_num.find(';') {
            &version_num[..semi_pos]
        } else {
            version_num
        };
        let version_num = version_num.trim();
        format!("{operator}{version_num}")
    };

    if version.is_empty() {
        return None;
    }

    // Calculate positions
    let name_start = line.find(name)? as u32;
    let name_end = name_start + name.len() as u32;

    // Find version (with operator) position in the original line
    let version_start = line.find(&version)? as u32;
    let version_end = version_start + version.len() as u32;

    Some(Dependency {
        name: name.to_string(),
        version,
        name_span: Span {
            line: line_num,
            line_start: name_start,
            line_end: name_end,
        },
        version_span: Span {
            line: line_num,
            line_start: version_start,
            line_end: version_end,
        },
        dev,
        optional: false,
        registry: None,
        resolved_version: None,
    })
}

/// Parse pyproject.toml format (PEP 621 + Poetry + Hatch)
fn parse_pyproject_toml(content: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();

    // Use taplo for parsing as it's more lenient and doesn't panic on malformed input
    let parsed = taplo::parser::parse(content);

    // If there are errors, skip this file
    if !parsed.errors.is_empty() {
        return dependencies;
    }

    let dom = parsed.into_dom();

    // PEP 621: [project.dependencies] array of strings
    let project = dom.get("project");
    if let Some(project_table) = project.as_table() {
        // [project.dependencies]
        let deps_node = project.get("dependencies");
        if let Some(deps_array) = deps_node.as_array() {
            let items = deps_array.items().read();
            for item in items.iter() {
                if let Some(dep_str) = item.as_str() {
                    let dep_str = dep_str.value();
                    if let Some((name, version)) = parse_pep508_dependency(dep_str)
                        && let Some(dep) =
                            find_dependency_position(content, &name, &version, false, false)
                    {
                        dependencies.push(dep);
                    }
                }
            }
        }

        // [project.optional-dependencies]
        let optional_node = project.get("optional-dependencies");
        if let Some(optional_deps) = optional_node.as_table() {
            let entries = optional_deps.entries().read();
            for (_group, deps_node) in entries.iter() {
                if let Some(deps_array) = deps_node.as_array() {
                    let items = deps_array.items().read();
                    for item in items.iter() {
                        if let Some(dep_str) = item.as_str() {
                            let dep_str = dep_str.value();
                            if let Some((name, version)) = parse_pep508_dependency(dep_str)
                                && let Some(dep) =
                                    find_dependency_position(content, &name, &version, true, true)
                            {
                                dependencies.push(dep);
                            }
                        }
                    }
                }
            }
        }

        // Suppress unused variable warning
        let _ = project_table;
    }

    // Poetry: [tool.poetry.dependencies] table
    let tool = dom.get("tool");
    let poetry = tool.get("poetry");
    if let Some(poetry_table) = poetry.as_table() {
        // [tool.poetry.dependencies]
        let deps_node = poetry.get("dependencies");
        if let Some(deps_table) = deps_node.as_table() {
            let entries = deps_table.entries().read();
            for (key, value) in entries.iter() {
                let name = key.value().to_string();
                // Skip python itself
                if name == "python" {
                    continue;
                }
                if let Some(version) = extract_poetry_version_taplo(value)
                    && let Some(dep) =
                        find_poetry_dependency_position(content, &name, &version, false)
                {
                    dependencies.push(dep);
                }
            }
        }

        // [tool.poetry.dev-dependencies] (Poetry < 1.2)
        let dev_deps_node = poetry.get("dev-dependencies");
        if let Some(deps_table) = dev_deps_node.as_table() {
            let entries = deps_table.entries().read();
            for (key, value) in entries.iter() {
                let name = key.value().to_string();
                if let Some(version) = extract_poetry_version_taplo(value)
                    && let Some(dep) =
                        find_poetry_dependency_position(content, &name, &version, true)
                {
                    dependencies.push(dep);
                }
            }
        }

        // [tool.poetry.group.dev.dependencies] (Poetry >= 1.2)
        let groups_node = poetry.get("group");
        if let Some(groups) = groups_node.as_table() {
            let group_entries = groups.entries().read();
            for (group_key, group_value) in group_entries.iter() {
                let group_name = group_key.value();
                let is_dev = group_name == "dev" || group_name == "test";
                if let Some(group_table) = group_value.as_table() {
                    let deps_node = group_value.get("dependencies");
                    if let Some(deps_table) = deps_node.as_table() {
                        let entries = deps_table.entries().read();
                        for (key, value) in entries.iter() {
                            let name = key.value().to_string();
                            if let Some(version) = extract_poetry_version_taplo(value)
                                && let Some(dep) = find_poetry_dependency_position(
                                    content, &name, &version, is_dev,
                                )
                            {
                                dependencies.push(dep);
                            }
                        }
                    }
                    // Suppress unused variable warning
                    let _ = group_table;
                }
            }
        }

        // Suppress unused variable warning
        let _ = poetry_table;
    }

    // PEP 735: [dependency-groups] table
    //
    // Each group is an array whose items are either:
    //   - a PEP 508 string specifier  → parsed exactly like [project.dependencies]
    //   - a table {include-group = "name"} → skipped here; since every group is
    //     iterated directly, the packages referenced by an include are already
    //     emitted when that group itself is processed, so no packages are missed.
    //
    // Unversioned items (e.g. "pytest" with no operator) produce no Dependency
    // because parse_pep508_dependency requires a version operator; there is
    // nothing to check without a version constraint.
    //
    // The spec assigns no "dev" semantics to group names, so dev = false for all.
    let dep_groups_node = dom.get("dependency-groups");
    if let Some(dep_groups_table) = dep_groups_node.as_table() {
        let group_entries = dep_groups_table.entries().read();
        for (_group_name, group_value) in group_entries.iter() {
            if let Some(items_array) = group_value.as_array() {
                let items = items_array.items().read();
                for item in items.iter() {
                    // String item: a PEP 508 dependency specifier
                    if let Some(dep_str) = item.as_str() {
                        let dep_str = dep_str.value();
                        if let Some((name, version)) = parse_pep508_dependency(dep_str)
                            && let Some(dep) =
                                find_dependency_position(content, &name, &version, false, false)
                        {
                            dependencies.push(dep);
                        }
                    }
                    // Table items ({include-group = "..."}) are intentionally skipped
                }
            }
        }
    }
    // Hatch: [tool.hatch.envs.<ENV_NAME>]
    // Both `dependencies` and `extra-dependencies` are PEP 508 string arrays.
    // Matrix overrides (e.g. [tool.hatch.envs.test.overrides.matrix.*.dependencies])
    // use a different inline-table value format and are out of scope.
    let hatch_envs = dom.get("tool").get("hatch").get("envs");
    collect_hatch_env_deps(&hatch_envs, content, &mut dependencies);

    dependencies
}

/// Parse a standalone `hatch.toml` file.
///
/// The project-level Hatch config is always stored in a file named `hatch.toml`.
/// In this format the envs table lives at the top level under `envs`
/// (no `tool.hatch` prefix as in pyproject.toml).
fn parse_hatch_toml(content: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();

    let parsed = taplo::parser::parse(content);
    if !parsed.errors.is_empty() {
        return dependencies;
    }

    let dom = parsed.into_dom();
    let envs_node = dom.get("envs");
    collect_hatch_env_deps(&envs_node, content, &mut dependencies);

    dependencies
}

/// Collect `dependencies` and `extra-dependencies` from a hatch envs Node.
///
/// `envs_node` is the taplo node for the envs table:
/// - `dom["tool"]["hatch"]["envs"]` when called from `parse_pyproject_toml`
/// - `dom["envs"]`                  when called from `parse_hatch_toml`
///
/// Flags set on every collected dependency:
/// - `dev = true`       — hatch env deps are extras layered on top of
///   `[project.dependencies]`; they are always dev/test tooling.
/// - `optional = false` — they are unconditionally installed when the env is
///   activated; they are not PEP 508 optional extras (project features/extras).
///
/// Context-formatted strings (e.g. `"{root:parent:uri}/pkg"`, `"{env:PKG:default}"`)
/// contain no PEP 508 version operator, so `parse_pep508_dependency` returns `None`
/// and they are silently skipped — identical to the treatment of unversioned plain
/// strings.
///
/// Env inheritance (`template` option) is not followed; only deps declared directly
/// in each env are emitted. This mirrors the treatment of Poetry group inheritance.
///
/// The `dependency-groups` key inside a hatch env is a reference to groups already
/// parsed elsewhere (e.g. from a `[dependency-groups]` table in the same file);
/// following those references here would produce duplicate entries.
fn collect_hatch_env_deps(
    envs_node: &taplo::dom::Node,
    content: &str,
    dependencies: &mut Vec<Dependency>,
) {
    let Some(envs_table) = envs_node.as_table() else {
        return;
    };
    let env_entries = envs_table.entries().read();
    for (_env_name, env_value) in env_entries.iter() {
        for key in ["dependencies", "extra-dependencies"] {
            let deps_node = env_value.get(key);
            let Some(deps_array) = deps_node.as_array() else {
                continue;
            };
            let items = deps_array.items().read();
            for item in items.iter() {
                let Some(dep_str) = item.as_str() else {
                    continue;
                };
                let dep_str = dep_str.value();
                if let Some((name, version)) = parse_pep508_dependency(dep_str)
                    && let Some(dep) =
                        find_dependency_position(content, &name, &version, true, false)
                {
                    dependencies.push(dep);
                }
            }
        }
    }
}

/// Parse PEP 508 dependency string: "package>=1.0.0" or "package[extra]>=1.0.0"
fn parse_pep508_dependency(dep_str: &str) -> Option<(String, String)> {
    let trimmed = dep_str.trim();

    // Remove environment markers
    let without_markers = if let Some(semi_pos) = trimmed.find(';') {
        &trimmed[..semi_pos]
    } else {
        trimmed
    };
    let without_markers = without_markers.trim();

    // Find version operator
    let operators = ["===", "==", ">=", "<=", "!=", "~=", ">", "<"];
    let mut op_pos = None;
    let mut op_len = 0;

    for op in &operators {
        if let Some(pos) = without_markers.find(op)
            && (op_pos.is_none() || pos < op_pos.unwrap())
        {
            op_pos = Some(pos);
            op_len = op.len();
        }
    }

    let op_pos = op_pos?;

    // Extract name (handle extras)
    let name_part = &without_markers[..op_pos];
    let name = if let Some(bracket_pos) = name_part.find('[') {
        &name_part[..bracket_pos]
    } else {
        name_part
    };
    let name = name.trim();

    // Extract version (including operator, to align with requirements.txt behavior)
    let operator = &without_markers[op_pos..op_pos + op_len];
    let version_part = &without_markers[op_pos + op_len..];
    let version_num = if let Some(comma_pos) = version_part.find(',') {
        &version_part[..comma_pos]
    } else {
        version_part
    };
    let version_num = version_num.trim();

    if name.is_empty() || version_num.is_empty() {
        return None;
    }

    Some((name.to_string(), format!("{operator}{version_num}")))
}

/// Extract version from Poetry dependency value (using taplo Node)
fn extract_poetry_version_taplo(value: &taplo::dom::Node) -> Option<String> {
    // Simple string value: flask = "^2.0.0"
    if let Some(s) = value.as_str() {
        return Some(s.value().to_string());
    }

    // Table value: flask = { version = "^2.0.0", ... }
    if let Some(t) = value.as_table()
        && let Some(version_node) = t.get("version")
        && let Some(version_str) = version_node.as_str()
    {
        return Some(version_str.value().to_string());
    }

    None
}

/// Find position of a dependency in PEP 621 format (array of strings)
fn find_dependency_position(
    content: &str,
    name: &str,
    version: &str,
    dev: bool,
    optional: bool,
) -> Option<Dependency> {
    for (line_idx, line) in content.lines().enumerate() {
        // Look for the dependency string in an array
        if line.contains(name) && line.contains(version) {
            // Check it's likely a dependency line (contains quotes and version operator)
            if line.contains('"') || line.contains('\'') {
                let line_num = line_idx as u32;

                // Find name position
                let name_start = line.find(name)? as u32;
                let name_end = name_start + name.len() as u32;

                // Find version position
                let version_start = line.find(version)? as u32;
                let version_end = version_start + version.len() as u32;

                return Some(Dependency {
                    name: name.to_string(),
                    version: version.to_string(),
                    name_span: Span {
                        line: line_num,
                        line_start: name_start,
                        line_end: name_end,
                    },
                    version_span: Span {
                        line: line_num,
                        line_start: version_start,
                        line_end: version_end,
                    },
                    dev,
                    optional,
                    registry: None,
                    resolved_version: None,
                });
            }
        }
    }
    None
}

/// Find position of a Poetry dependency (table format)
fn find_poetry_dependency_position(
    content: &str,
    name: &str,
    version: &str,
    dev: bool,
) -> Option<Dependency> {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Poetry format: name = "version" or name = { version = "..." }
        if trimmed.starts_with(name) && trimmed.contains('=') {
            // Check this line contains the version
            if line.contains(version) {
                let line_num = line_idx as u32;

                // Find name position
                let name_start = line.find(name)? as u32;
                let name_end = name_start + name.len() as u32;

                // Find version position (inside quotes)
                let version_start = line.find(version)? as u32;
                let version_end = version_start + version.len() as u32;

                return Some(Dependency {
                    name: name.to_string(),
                    version: version.to_string(),
                    name_span: Span {
                        line: line_num,
                        line_start: name_start,
                        line_end: name_end,
                    },
                    version_span: Span {
                        line: line_num,
                        line_start: version_start,
                        line_end: version_end,
                    },
                    dev,
                    optional: false,
                    registry: None,
                    resolved_version: None,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requirements_simple() {
        let parser = PythonParser::new();
        let content = r#"
flask==2.0.0
requests>=2.25.0
django~=4.0
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, "==2.0.0");

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, ">=2.25.0");

        let django = deps.iter().find(|d| d.name == "django").unwrap();
        assert_eq!(django.version, "~=4.0");
    }

    #[test]
    fn test_requirements_with_extras() {
        let parser = PythonParser::new();
        let content = "uvicorn[standard]>=0.20.0";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "uvicorn");
        assert_eq!(deps[0].version, ">=0.20.0");
    }

    #[test]
    fn test_requirements_with_comments() {
        let parser = PythonParser::new();
        let content = r#"
# This is a comment
flask==2.0.0  # inline comment
# Another comment
requests>=2.25.0
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_requirements_skip_special() {
        let parser = PythonParser::new();
        let content = r#"
-r other.txt
-e git+https://github.com/user/repo.git
--index-url https://pypi.org/simple
flask==2.0.0
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "flask");
    }

    #[test]
    fn test_pyproject_pep621() {
        let parser = PythonParser::new();
        let content = r#"
[project]
name = "myproject"
dependencies = [
    "flask>=2.0.0",
    "requests~=2.25.0",
]

[project.optional-dependencies]
dev = [
    "pytest>=7.0.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, ">=2.0.0");
        assert!(!flask.dev);

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.version, ">=7.0.0");
        assert!(pytest.dev);
    }

    #[test]
    fn test_pyproject_poetry() {
        let parser = PythonParser::new();
        let content = r#"
[tool.poetry]
name = "myproject"

[tool.poetry.dependencies]
python = "^3.9"
flask = "^2.0.0"
requests = { version = "^2.25.0", optional = true }

[tool.poetry.dev-dependencies]
pytest = "^7.0.0"
"#;
        let deps = parser.parse(content);
        // Should have flask, requests, pytest (python is skipped)
        assert_eq!(deps.len(), 3);

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, "^2.0.0");
        assert!(!flask.dev);

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.version, "^7.0.0");
        assert!(pytest.dev);
    }

    #[test]
    fn test_version_position() {
        let parser = PythonParser::new();
        let content = "flask==2.0.0";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);

        let dep = &deps[0];
        assert_eq!(dep.version, "==2.0.0");
        assert_eq!(dep.name_span.line, 0);
        assert_eq!(dep.name_span.line_start, 0);
        assert_eq!(dep.name_span.line_end, 5);
        // version_start now includes the operator "=="
        assert_eq!(dep.version_span.line, 0);
        assert_eq!(dep.version_span.line_start, 5);
        assert_eq!(dep.version_span.line_end, 12);
    }

    #[test]
    fn test_requirements_with_project_extra_not_toml() {
        // Ensure packages with [project] as extras don't trigger TOML parsing
        let parser = PythonParser::new();
        let content = r#"
mypkg[project]==1.2.0
otherpkg[tool.poetry]>=2.0
flask>=2.0.0
"#;
        let deps = parser.parse(content);
        // Should be parsed as requirements.txt, not pyproject.toml
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "mypkg"));
        assert!(deps.iter().any(|d| d.name == "otherpkg"));
        assert!(deps.iter().any(|d| d.name == "flask"));
    }

    #[test]
    fn test_is_pyproject_toml_detection() {
        // Valid pyproject.toml patterns - [project] and subsections
        assert!(is_pyproject_toml("[project]\nname = \"test\""));
        assert!(is_pyproject_toml("  [project]  \nname = \"test\""));
        assert!(is_pyproject_toml(
            "[project.dependencies]\nflask = \">=2.0\""
        ));
        assert!(is_pyproject_toml(
            "[project.optional-dependencies]\ndev = []"
        ));

        // Valid patterns with inline comments
        assert!(is_pyproject_toml(
            "[project] # main section\nname = \"test\""
        ));
        assert!(is_pyproject_toml(
            "[project.dependencies]  # deps\nflask = \"1.0\""
        ));

        // Valid [tool.poetry] patterns
        assert!(is_pyproject_toml("[tool.poetry]\nname = \"test\""));
        assert!(is_pyproject_toml(
            "[tool.poetry.dependencies]\npython = \"^3.9\""
        ));
        assert!(is_pyproject_toml(
            "[tool.poetry] # comment\nname = \"test\""
        ));

        // Valid [dependency-groups] patterns (PEP 735)
        assert!(is_pyproject_toml(
            "[dependency-groups]\ntest = [\"pytest\"]"
        ));
        assert!(is_pyproject_toml(
            "[dependency-groups] # comment\ntest = []"
        ));
        assert!(is_pyproject_toml("  [dependency-groups]  \ntest = []"));
        // Valid [tool.hatch] patterns
        assert!(is_pyproject_toml(
            "[tool.hatch.envs.test]\ndependencies = []"
        ));
        assert!(is_pyproject_toml(
            "[tool.hatch.envs.default]\ndependencies = []"
        ));
        assert!(is_pyproject_toml("[tool.hatch] # comment\nversion = {}"));
        assert!(is_pyproject_toml(
            "[tool.hatch.envs.test.scripts]\ntest = \"pytest\""
        ));

        // Invalid patterns (should not trigger TOML parsing)
        assert!(!is_pyproject_toml("mypkg[project]==1.2"));
        assert!(!is_pyproject_toml("pkg[tool.poetry]>=1.0"));
        assert!(!is_pyproject_toml("pkg[tool.hatch]>=1.0"));
        assert!(!is_pyproject_toml("[projects]\nname = \"test\"")); // not [project]
        assert!(!is_pyproject_toml("[projectx]\nname = \"test\"")); // not [project] or [project.*]
        assert!(!is_pyproject_toml("[tool.poetryextra]\nname = \"test\"")); // not [tool.poetry...]
        assert!(!is_pyproject_toml("[tool.hatchextra]\nname = \"test\"")); // not [tool.hatch...]
        assert!(!is_pyproject_toml("flask>=2.0.0\nrequests>=2.25.0"));
        assert!(!is_pyproject_toml("[dependency-groupsx]\ntest = []")); // not [dependency-groups]
    }

    #[test]
    fn test_pyproject_dependency_groups() {
        // Covers:
        //   - file with ONLY [dependency-groups] (non-package project, no [project] block)
        //   - multiple groups, multiple versioned deps per group
        //   - {include-group = "..."} table items are silently skipped (all groups are
        //     iterated directly, so no package is ever missed via this skip)
        //   - unversioned items ("bare-package" without operator) produce no Dependency
        //   - dev = false for all groups (spec assigns no dev semantics to group names)
        let parser = PythonParser::new();
        let content = r#"
[dependency-groups]
test = ["pytest>=7.0.0", "coverage>=7.0.0"]
typing = ["mypy>=1.0.0", {include-group = "test"}, "types-requests>=2.0.0"]
typing-test = [{include-group = "typing"}, {include-group = "test"}, "useful-types>=1.0.0"]
unversioned = ["bare-package"]
"#;
        let deps = parser.parse(content);

        // test: 2, typing: 2 (include-group skipped), typing-test: 1 (both include-groups skipped)
        // unversioned: 0 (no version operator → parse_pep508_dependency returns None)
        assert_eq!(deps.len(), 5);

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.version, ">=7.0.0");
        assert!(!pytest.dev);

        let coverage = deps.iter().find(|d| d.name == "coverage").unwrap();
        assert_eq!(coverage.version, ">=7.0.0");
        assert!(!coverage.dev);

        let mypy = deps.iter().find(|d| d.name == "mypy").unwrap();
        assert_eq!(mypy.version, ">=1.0.0");
        assert!(!mypy.dev);

        let types_requests = deps.iter().find(|d| d.name == "types-requests").unwrap();
        assert_eq!(types_requests.version, ">=2.0.0");
        assert!(!types_requests.dev);

        let useful_types = deps.iter().find(|d| d.name == "useful-types").unwrap();
        assert_eq!(useful_types.version, ">=1.0.0");
        assert!(!useful_types.dev);

        // bare-package has no version operator → must not appear
        assert!(!deps.iter().any(|d| d.name == "bare-package"));
    }

    #[test]
    fn test_is_hatch_toml_detection() {
        // Valid: top-level [envs.<name>] section
        assert!(is_hatch_toml("[envs.test]\ndependencies = []"));
        assert!(is_hatch_toml("[envs.default]\ndependencies = []"));
        // Sub-tables of an env are also valid triggers
        assert!(is_hatch_toml("[envs.test.scripts]\ntest = \"pytest\""));
        // Inline comment after closing bracket
        assert!(is_hatch_toml(
            "[envs.test] # my test env\ndependencies = []"
        ));
        // Leading whitespace on header line
        assert!(is_hatch_toml("  [envs.test]  \ndependencies = []"));

        // Invalid: bare [envs] without a name
        assert!(!is_hatch_toml("[envs]\ndependencies = []"));
        // Invalid: different top-level key
        assert!(!is_hatch_toml("[envsx.test]\ndependencies = []"));
        // Invalid: content that would be pyproject.toml
        assert!(!is_hatch_toml("[project]\nname = \"test\""));
        // Invalid: requirements.txt style
        assert!(!is_hatch_toml("flask>=2.0.0\nrequests>=2.25.0"));
        // Invalid: env name is part of a value, not a section header
        assert!(!is_hatch_toml("template = \"[envs.default]\""));
    }

    #[test]
    fn test_pyproject_hatch_deps() {
        // Basic [tool.hatch.envs.*] with dependencies
        let parser = PythonParser::new();
        let content = r#"
[project]
name = "myproject"
version = "1.0.0"

[tool.hatch.envs.test]
dependencies = [
    "pytest>=7.0.0",
    "coverage>=6.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.version, ">=7.0.0");
        assert!(pytest.dev);
        assert!(!pytest.optional);

        let coverage = deps.iter().find(|d| d.name == "coverage").unwrap();
        assert_eq!(coverage.version, ">=6.0");
        assert!(coverage.dev);
        assert!(!coverage.optional);
    }

    #[test]
    fn test_pyproject_hatch_extra_deps() {
        // extra-dependencies in a hatch env
        let parser = PythonParser::new();
        let content = r#"
[project]
name = "myproject"

[tool.hatch.envs.default]
dependencies = [
    "foo>=1.0",
]

[tool.hatch.envs.experimental]
extra-dependencies = [
    "baz>=2.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let foo = deps.iter().find(|d| d.name == "foo").unwrap();
        assert_eq!(foo.version, ">=1.0");
        assert!(foo.dev);
        assert!(!foo.optional);

        let baz = deps.iter().find(|d| d.name == "baz").unwrap();
        assert_eq!(baz.version, ">=2.0");
        assert!(baz.dev);
        assert!(!baz.optional);
    }

    #[test]
    fn test_pyproject_hatch_multiple_envs() {
        // Several named envs each contributing deps; all dev=true, optional=false
        let parser = PythonParser::new();
        let content = r#"
[project]
name = "myproject"

[tool.hatch.envs.default]
dependencies = ["requests>=2.28.0"]

[tool.hatch.envs.test]
dependencies = [
    "pytest>=7.0.0",
    "coverage[toml]>=6.0",
]

[tool.hatch.envs.lint]
dependencies = [
    "ruff>=0.1.0",
    "mypy>=1.0.0",
]
extra-dependencies = [
    "types-requests>=2.28.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 6);
        assert!(deps.iter().all(|d| d.dev));
        assert!(deps.iter().all(|d| !d.optional));

        assert!(deps.iter().any(|d| d.name == "requests"));
        assert!(deps.iter().any(|d| d.name == "pytest"));
        assert!(deps.iter().any(|d| d.name == "coverage"));
        assert!(deps.iter().any(|d| d.name == "ruff"));
        assert!(deps.iter().any(|d| d.name == "mypy"));
        assert!(deps.iter().any(|d| d.name == "types-requests"));
    }

    #[test]
    fn test_pyproject_hatch_no_version() {
        // Bare package name without a version operator is silently dropped
        let parser = PythonParser::new();
        let content = r#"
[tool.hatch.envs.test]
dependencies = [
    "bare-package",
    "pytest>=7.0.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "pytest");
        assert!(!deps.iter().any(|d| d.name == "bare-package"));
    }

    #[test]
    fn test_pyproject_hatch_context_formatted() {
        // Context-formatted strings (hatch-specific, no PEP 508 version operator) are
        // silently skipped.  A versioned dep on the same env is still collected.
        let parser = PythonParser::new();
        let content = r#"
[tool.hatch.envs.test]
dependencies = [
    "example-project @ {root:parent:parent:uri}/example-project",
    "pytest>=7.0.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "pytest");
    }

    #[test]
    fn test_pyproject_hatch_combined_pep621() {
        // pyproject.toml with [project.dependencies] (dev=false) and
        // [tool.hatch.envs.*] (dev=true, optional=false) in the same file.
        let parser = PythonParser::new();
        let content = r#"
[project]
name = "myproject"
dependencies = [
    "flask>=2.0.0",
    "requests~=2.25.0",
]

[project.optional-dependencies]
extras = [
    "redis>=4.0.0",
]

[tool.hatch.envs.test]
dependencies = [
    "pytest>=7.0.0",
    "coverage>=6.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 5);

        // Project deps: dev=false, optional=false
        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert!(!flask.dev);
        assert!(!flask.optional);

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert!(!requests.dev);
        assert!(!requests.optional);

        // Optional dep: dev=true, optional=true
        let redis = deps.iter().find(|d| d.name == "redis").unwrap();
        assert!(redis.dev);
        assert!(redis.optional);

        // Hatch env deps: dev=true, optional=false
        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert!(pytest.dev);
        assert!(!pytest.optional);

        let coverage = deps.iter().find(|d| d.name == "coverage").unwrap();
        assert!(coverage.dev);
        assert!(!coverage.optional);
    }

    #[test]
    fn test_hatch_toml_basic() {
        // Standalone hatch.toml — envs at top level under [envs.*]
        let parser = PythonParser::new();
        let content = r#"
[envs.default]
dependencies = [
    "mypy>=1.0.0",
]

[envs.test]
dependencies = [
    "pytest>=7.0.0",
    "coverage>=6.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().all(|d| d.dev));
        assert!(deps.iter().all(|d| !d.optional));

        assert!(deps.iter().any(|d| d.name == "mypy"));
        assert!(deps.iter().any(|d| d.name == "pytest"));
        assert!(deps.iter().any(|d| d.name == "coverage"));
    }

    #[test]
    fn test_hatch_toml_extra_deps() {
        // extra-dependencies in standalone hatch.toml
        let parser = PythonParser::new();
        let content = r#"
[envs.default]
dependencies = [
    "foo>=1.0",
    "bar>=2.0",
]

[envs.experimental]
extra-dependencies = [
    "baz>=3.0",
]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "foo"));
        assert!(deps.iter().any(|d| d.name == "bar"));
        assert!(deps.iter().any(|d| d.name == "baz"));
        assert!(deps.iter().all(|d| d.dev));
        assert!(deps.iter().all(|d| !d.optional));
    }

    #[test]
    fn test_pyproject_poetry_groups_dev() {
        let content = r#"
[tool.poetry]
name = "x"
version = "0.1.0"

[tool.poetry.group.dev.dependencies]
black = "^23.0.0"
ruff = "^0.1.0"
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2, "expected 2 deps, got {:?}", deps);
        let black = deps.iter().find(|d| d.name == "black").expect("black missing");
        assert!(black.dev, "black in [group.dev] must be dev=true");
        assert!(!black.optional);
        let ruff = deps.iter().find(|d| d.name == "ruff").expect("ruff missing");
        assert!(ruff.dev);
    }

    #[test]
    fn test_pyproject_poetry_groups_test() {
        let content = r#"
[tool.poetry]
name = "x"
version = "0.1.0"

[tool.poetry.group.test.dependencies]
pytest = "^7.0.0"
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        let pytest = deps.iter().find(|d| d.name == "pytest").expect("pytest missing");
        assert!(pytest.dev, "pytest in [group.test] must be dev=true");
    }

    #[test]
    fn test_pyproject_poetry_groups_custom() {
        let content = r#"
[tool.poetry]
name = "x"
version = "0.1.0"

[tool.poetry.group.docs.dependencies]
mkdocs = "^1.5.0"
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        let mkdocs = deps.iter().find(|d| d.name == "mkdocs").expect("mkdocs missing");
        assert!(!mkdocs.dev, "mkdocs in [group.docs] must be dev=false");
        assert!(!mkdocs.optional);
    }

    #[test]
    fn test_pyproject_poetry_table_format() {
        let content = r#"
[tool.poetry.dependencies]
python = "^3.9"
requests = { version = "^2.28.0", optional = true }
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        let req = deps.iter().find(|d| d.name == "requests").expect("requests missing");
        assert_eq!(req.version, "^2.28.0");
    }

    #[test]
    fn test_pyproject_pep621_dynamic_safe() {
        let content = r#"
[project]
name = "x"
version = "0.1.0"
dynamic = ["dependencies"]
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 0, "dynamic deps should yield no Dependency items");
    }

    #[test]
    fn test_pyproject_environment_markers() {
        let content = r#"
[project]
name = "x"
version = "0.1.0"
dependencies = [
    "pytest>=7.0;python_version>='3.8'",
]
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        let pytest = deps.iter().find(|d| d.name == "pytest").expect("pytest missing");
        assert_eq!(pytest.version, ">=7.0", "marker must be stripped");
    }

    #[test]
    fn test_pyproject_mixed_all_sections() {
        let content = r#"
[project]
name = "x"
version = "0.1.0"
dependencies = ["requests>=2.28.0"]

[project.optional-dependencies]
docs = ["mkdocs>=1.5.0"]

[dependency-groups]
test = ["pytest>=7.0.0"]

[tool.hatch.envs.lint]
dependencies = ["ruff>=0.1.0"]
"#;
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        assert!(deps.iter().any(|d| d.name == "requests"));
        assert!(deps.iter().any(|d| d.name == "mkdocs" && d.optional));
        assert!(deps.iter().any(|d| d.name == "pytest"));
        assert!(deps.iter().any(|d| d.name == "ruff" && d.dev));
        assert_eq!(deps.len(), 4, "expected 4 deps total, got {:?}", deps);
    }

    #[test]
    fn test_pyproject_pep621_position_accuracy() {
        let content = "[project]\nname = \"x\"\nversion = \"0.1.0\"\ndependencies = [\n    \"requests>=2.28.0\",\n]\n";
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        let dep = &deps[0];
        assert_eq!(dep.name, "requests");
        assert_eq!(dep.version, ">=2.28.0");
        assert_eq!(dep.name_span.line, 4, "name should be on the line containing the array item");
        assert_eq!(dep.version_span.line, 4);
        assert!(dep.name_span.line_end > dep.name_span.line_start);
        assert!(dep.version_span.line_end > dep.version_span.line_start);
    }

    #[test]
    fn test_pyproject_poetry_position_accuracy() {
        let content = "[tool.poetry.dependencies]\npython = \"^3.9\"\nrequests = \"^2.28.0\"\n";
        let parser = PythonParser::new();
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1, "only requests, python is skipped");
        let dep = &deps[0];
        assert_eq!(dep.name, "requests");
        assert_eq!(dep.version, "^2.28.0");
        assert_eq!(dep.name_span.line, 2);
        assert_eq!(dep.version_span.line, 2);
        assert!(dep.name_span.line_end > dep.name_span.line_start);
        assert!(dep.version_span.line_end > dep.version_span.line_start);
    }
}
