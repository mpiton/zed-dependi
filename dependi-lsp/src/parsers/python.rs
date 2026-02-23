//! Parser for Python dependency files (requirements.txt, constraints.txt, pyproject.toml)

use super::{Dependency, Parser};

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
        // Detect file type based on content
        // Only parse as TOML if it contains valid pyproject.toml section headers
        // Use line-anchored detection to avoid false positives like "mypkg[project]==1.2"
        if is_pyproject_toml(content) {
            parse_pyproject_toml(content)
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
    let version = if let Some(op_pos) = version_op_pos {
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
        format!("{}{}", operator, version_num)
    } else {
        // No version specified
        return None;
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
        line: line_num,
        name_start,
        name_end,
        version_start,
        version_end,
        dev,
        optional: false,
        registry: None,
    })
}

/// Parse pyproject.toml format (PEP 621 + Poetry)
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
                        && let Some(dep) = find_dependency_position(content, &name, &version, false)
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
                                    find_dependency_position(content, &name, &version, true)
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

    dependencies
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

    Some((name.to_string(), format!("{}{}", operator, version_num)))
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
                    line: line_num,
                    name_start,
                    name_end,
                    version_start,
                    version_end,
                    dev,
                    optional: dev, // optional-dependencies are optional
                    registry: None,
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
                    line: line_num,
                    name_start,
                    name_end,
                    version_start,
                    version_end,
                    dev,
                    optional: false,
                    registry: None,
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
        assert_eq!(dep.name_start, 0);
        assert_eq!(dep.name_end, 5);
        // version_start now includes the operator "=="
        assert_eq!(dep.version_start, 5);
        assert_eq!(dep.version_end, 12);
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

        // Invalid patterns (should not trigger TOML parsing)
        assert!(!is_pyproject_toml("mypkg[project]==1.2"));
        assert!(!is_pyproject_toml("pkg[tool.poetry]>=1.0"));
        assert!(!is_pyproject_toml("[projects]\nname = \"test\"")); // not [project]
        assert!(!is_pyproject_toml("[projectx]\nname = \"test\"")); // not [project] or [project.*]
        assert!(!is_pyproject_toml("[tool.poetryextra]\nname = \"test\"")); // not [tool.poetry...]
        assert!(!is_pyproject_toml("flask>=2.0.0\nrequests>=2.25.0"));
    }
}
