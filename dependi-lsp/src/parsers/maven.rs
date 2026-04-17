//! Maven (pom.xml) parser for Java projects.
//!
//! Parses direct dependencies declared in `pom.xml` files, including
//! `<dependencyManagement>`, with two passes:
//! 1. Collect `<properties>` for variable substitution (`${...}`).
//! 2. Extract dependencies and substitute property references.
//!
//! The dependency `name` uses the Maven convention `groupId:artifactId`
//! (matching OSV.dev and the mvnrepository.com URL scheme).
//!
//! Unsupported in this MVP (detected but not resolved):
//! - Parent POM inheritance
//! - BOM (`<scope>import</scope>`) resolution from remote POMs
//! - Plugin dependencies

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::parsers::{Dependency, Parser};

/// Parser for Maven `pom.xml` files.
#[derive(Default)]
pub struct MavenParser;

impl MavenParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for MavenParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let properties = extract_properties(content);
        extract_dependencies(content, &properties)
    }
}

/// Precomputed byte-offset → (line, column) mapping for a source string.
/// Lines are 0-indexed, columns are character-based within each line.
fn line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Convert a byte offset to (line, column), both 0-indexed.
fn offset_to_position(offsets: &[usize], byte_offset: usize) -> (u32, u32) {
    let line_idx = match offsets.binary_search(&byte_offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let line_start = offsets[line_idx];
    let col = byte_offset.saturating_sub(line_start);
    (line_idx as u32, col as u32)
}

/// Pass 1: collect `<properties>` entries (name → value) from the pom.
fn extract_properties(content: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut out = HashMap::new();
    let mut depth_stack: Vec<Vec<u8>> = Vec::new();
    let mut current_key: Option<String> = None;

    loop {
        match reader.read_event() {
            Err(_) => return HashMap::new(),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                // We want properties that are a direct child of <properties>
                // which is itself a child of <project>. Path: project > properties > <key>.
                // depth_stack represents the path of open elements excluding the current.
                let is_key = depth_stack.last().map(|v| v.as_slice()) == Some(b"properties");
                if is_key {
                    // Only accept if the grandparent is project.
                    if depth_stack.len() >= 2
                        && depth_stack[depth_stack.len() - 2] == b"project"
                    {
                        if let Ok(s) = std::str::from_utf8(&name) {
                            current_key = Some(s.to_string());
                        }
                    }
                }
                depth_stack.push(name);
            }
            Ok(Event::Text(e)) => {
                if let Some(ref key) = current_key {
                    if let Ok(text) = e.decode() {
                        out.insert(key.clone(), text.into_owned());
                    }
                }
            }
            Ok(Event::End(_)) => {
                depth_stack.pop();
                current_key = None;
            }
            _ => {}
        }
    }

    out
}

/// Pass 2: extract dependencies from `<dependencies>` and
/// `<dependencyManagement><dependencies>`, substituting `${property}` placeholders.
fn extract_dependencies(
    content: &str,
    properties: &HashMap<String, String>,
) -> Vec<Dependency> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let offsets = line_offsets(content);
    let mut out = Vec::new();

    // State: track which element we're inside.
    let mut in_dependencies = false;
    let mut in_dep_mgmt = false;
    let mut in_plugins = false;
    let mut in_dependency = false;
    let mut current_tag: Option<Vec<u8>> = None;

    // Current dependency accumulator
    let mut cur_group: Option<String> = None;
    let mut cur_artifact: Option<String> = None;
    let mut cur_version: Option<String> = None;
    let mut cur_version_span: Option<(usize, usize)> = None; // byte offsets into content
    let mut cur_scope: Option<String> = None;
    let mut cur_optional = false;

    loop {
        match reader.read_event() {
            Err(_) => return vec![], // invalid XML → empty result
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"dependencies" => in_dependencies = true,
                    b"dependencyManagement" => in_dep_mgmt = true,
                    b"plugins" | b"pluginManagement" => in_plugins = true,
                    b"dependency" if (in_dependencies || in_dep_mgmt) && !in_plugins => {
                        in_dependency = true;
                        cur_group = None;
                        cur_artifact = None;
                        cur_version = None;
                        cur_version_span = None;
                        cur_scope = None;
                        cur_optional = false;
                    }
                    _ => {}
                }
                current_tag = Some(name);
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"dependencies" => in_dependencies = false,
                    b"dependencyManagement" => in_dep_mgmt = false,
                    b"plugins" | b"pluginManagement" => in_plugins = false,
                    b"dependency" if in_dependency => {
                        in_dependency = false;
                        if let (Some(g), Some(a)) = (cur_group.take(), cur_artifact.take()) {
                            let raw_version = cur_version.take().unwrap_or_default();
                            let version = substitute(&raw_version, properties);
                            let scope = cur_scope.take().unwrap_or_default();
                            let dev = scope == "test" || scope == "provided";

                            let (line, version_start, version_end) = match cur_version_span.take() {
                                Some((s, e_)) => {
                                    let (l, col_s) = offset_to_position(&offsets, s);
                                    let (_, col_e) = offset_to_position(&offsets, e_);
                                    (l, col_s, col_e)
                                }
                                None => (0, 0, 0),
                            };

                            if !a.is_empty() && !g.is_empty() {
                                out.push(Dependency {
                                    name: format!("{g}:{a}"),
                                    version,
                                    line,
                                    name_start: 0,
                                    name_end: 0,
                                    version_start,
                                    version_end,
                                    dev,
                                    optional: cur_optional,
                                    registry: None,
                                    resolved_version: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
                current_tag = None;
            }
            Ok(Event::Text(e)) => {
                if in_dependency {
                    let text = match e.decode() {
                        Ok(s) => s.into_owned(),
                        Err(_) => continue,
                    };
                    match current_tag.as_deref() {
                        Some(b"groupId") => cur_group = Some(text),
                        Some(b"artifactId") => cur_artifact = Some(text),
                        Some(b"version") => {
                            // Capture byte offsets of the text content.
                            let end = reader.buffer_position() as usize;
                            let start = end.saturating_sub(text.len());
                            cur_version_span = Some((start, end));
                            cur_version = Some(text);
                        }
                        Some(b"scope") => cur_scope = Some(text),
                        Some(b"optional") => cur_optional = text.trim() == "true",
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let _ = properties; // properties used in substitute(); silence if empty
    out
}

/// Substitute `${property}` placeholders in a version string with values from `properties`.
/// Unresolved placeholders are preserved verbatim.
fn substitute(raw: &str, properties: &HashMap<String, String>) -> String {
    if !raw.contains("${") || properties.is_empty() {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            let key = &after[..end];
            match properties.get(key) {
                Some(v) => out.push_str(v),
                None => {
                    // Preserve the original `${key}` placeholder.
                    out.push_str("${");
                    out.push_str(key);
                    out.push('}');
                }
            }
            rest = &after[end + 1..];
        } else {
            // Unterminated `${`; bail out as literal.
            out.push_str("${");
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_dependency() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>app</artifactId>
    <version>1.0.0</version>
    <dependencies>
        <dependency>
            <groupId>org.slf4j</groupId>
            <artifactId>slf4j-api</artifactId>
            <version>1.7.30</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1, "should parse one dependency");
        assert_eq!(deps[0].name, "org.slf4j:slf4j-api");
        assert_eq!(deps[0].version, "1.7.30");
        assert!(!deps[0].dev);
        assert!(!deps[0].optional);
    }

    #[test]
    fn test_parse_with_properties() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>app</artifactId>
    <version>1.0.0</version>
    <properties>
        <spring.version>6.1.0</spring.version>
    </properties>
    <dependencies>
        <dependency>
            <groupId>org.springframework</groupId>
            <artifactId>spring-core</artifactId>
            <version>${spring.version}</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "6.1.0");
    }

    #[test]
    fn test_parse_unresolved_property_preserved() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>${not.defined}</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "${not.defined}");
    }

    #[test]
    fn test_parse_scope_test_marked_as_dev() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>junit</groupId>
            <artifactId>junit</artifactId>
            <version>4.13.2</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].dev);
    }

    #[test]
    fn test_parse_scope_provided_marked_as_dev() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>javax.servlet</groupId>
            <artifactId>servlet-api</artifactId>
            <version>2.5</version>
            <scope>provided</scope>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].dev);
    }

    #[test]
    fn test_parse_optional() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>1.0</version>
            <optional>true</optional>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].optional);
    }

    #[test]
    fn test_parse_snapshot_version() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>2.0-SNAPSHOT</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "2.0-SNAPSHOT");
    }

    #[test]
    fn test_parse_dependency_management() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencyManagement>
        <dependencies>
            <dependency>
                <groupId>g</groupId>
                <artifactId>a</artifactId>
                <version>3.0</version>
            </dependency>
        </dependencies>
    </dependencyManagement>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1, "depMgmt deps with versions should be parsed");
        assert_eq!(deps[0].version, "3.0");
    }

    #[test]
    fn test_parse_plugin_dependencies_ignored() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <build>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-compiler-plugin</artifactId>
                <version>3.11.0</version>
                <dependencies>
                    <dependency>
                        <groupId>ignored</groupId>
                        <artifactId>ignored</artifactId>
                        <version>0.1</version>
                    </dependency>
                </dependencies>
            </plugin>
        </plugins>
    </build>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>1.0</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        // Only the top-level <dependencies>/<dependency> should be captured.
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "g:a");
    }

    #[test]
    fn test_parse_invalid_xml_returns_empty() {
        let parser = MavenParser::new();
        let bad = "<project><dependencies><dependency>";
        let deps = parser.parse(bad);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_position_tracking() {
        let parser = MavenParser::new();
        let pom = r#"<?xml version="1.0"?>
<project>
    <dependencies>
        <dependency>
            <groupId>g</groupId>
            <artifactId>a</artifactId>
            <version>1.2.3</version>
        </dependency>
    </dependencies>
</project>
"#;
        let deps = parser.parse(pom);
        assert_eq!(deps.len(), 1);
        // The version line should be zero-indexed; the exact line varies with the raw string,
        // so we just sanity-check it is non-zero and the span is reasonable.
        assert!(deps[0].line > 0, "line should be tracked (got {})", deps[0].line);
        assert!(
            deps[0].version_end > deps[0].version_start,
            "version span should be non-empty"
        );
    }
}
