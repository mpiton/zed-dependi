//! Parser for concrete NuGet items in .NET/MSBuild manifests.
//!
//! Supported items use `PackageReference`, `PackageVersion`, or
//! `GlobalPackageReference` with literal package names and versions.
//!
//! ```xml
//! <PackageReference Include="Serilog" Version="3.1.1" />
//! <PackageVersion Include="Serilog" Version="3.1.1" />
//! ```

use super::{Dependency, Parser, Span};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NugetItemKind {
    PackageReference,
    PackageVersion,
    GlobalPackageReference,
}

impl NugetItemKind {
    fn from_local_name(name: &[u8]) -> Option<Self> {
        match name {
            b"PackageReference" => Some(Self::PackageReference),
            b"PackageVersion" => Some(Self::PackageVersion),
            b"GlobalPackageReference" => Some(Self::GlobalPackageReference),
            _ => None,
        }
    }

    fn is_development(self) -> bool {
        self == Self::GlobalPackageReference
    }
}

#[derive(Debug)]
struct LocatedValue {
    value: String,
    absolute_start: usize,
    absolute_end: usize,
}

#[derive(Debug)]
struct PendingItem {
    kind: NugetItemKind,
    name: Option<LocatedValue>,
    version: Option<LocatedValue>,
}

impl PendingItem {
    fn from_event(
        kind: NugetItemKind,
        event: &BytesStart<'_>,
        event_start: usize,
    ) -> Result<Self, ()> {
        let include = located_attribute(event, event_start, b"Include")?;
        let name = if include.is_some() {
            include
        } else if kind == NugetItemKind::PackageVersion {
            located_attribute(event, event_start, b"Update")?
        } else {
            None
        };

        Ok(Self {
            kind,
            name,
            version: located_attribute(event, event_start, b"Version")?,
        })
    }
}

/// Parser for NuGet items in .NET/MSBuild manifests.
///
/// # Examples
///
/// ```
/// use dependi_lsp::parsers::Parser;
/// use dependi_lsp::parsers::csharp::CsharpParser;
/// let parser = CsharpParser::new();
/// let content = r#"<Project><ItemGroup><PackageReference Include="Serilog" Version="3.1.1" /></ItemGroup></Project>"#;
/// let deps = parser.parse(content);
/// assert_eq!(deps.len(), 1);
/// assert_eq!(deps[0].name, "Serilog");
/// assert_eq!(deps[0].version, "3.1.1");
/// ```
#[derive(Debug, Default)]
pub struct CsharpParser;

impl CsharpParser {
    /// Creates a new [`CsharpParser`] instance.
    pub fn new() -> Self {
        Self
    }
}

impl Parser for CsharpParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let newline_starts = newline_starts(content);
        let mut reader = Reader::from_str(content);
        let mut dependencies = Vec::new();

        loop {
            match reader.read_event() {
                Ok(Event::Empty(event)) => {
                    let Some(kind) = NugetItemKind::from_local_name(event.local_name().as_ref())
                    else {
                        continue;
                    };
                    let Some(event_start) =
                        event_content_start(reader.buffer_position(), &event, true)
                    else {
                        return Vec::new();
                    };
                    let pending = match PendingItem::from_event(kind, &event, event_start) {
                        Ok(pending) => pending,
                        Err(()) => return Vec::new(),
                    };
                    if let Some(dependency) = build_dependency(pending, content, &newline_starts) {
                        dependencies.push(dependency);
                    }
                }
                Ok(Event::Eof) => return dependencies,
                Ok(_) => {}
                Err(_) => return Vec::new(),
            }
        }
    }
}

fn build_dependency(
    pending: PendingItem,
    content: &str,
    newline_starts: &[usize],
) -> Option<Dependency> {
    let name = pending.name?;
    let version = pending.version?;
    let name_span = span_from_absolute(
        content,
        newline_starts,
        name.absolute_start,
        name.absolute_end,
    )?;
    let version_span = span_from_absolute(
        content,
        newline_starts,
        version.absolute_start,
        version.absolute_end,
    )?;
    Some(Dependency {
        name: name.value,
        version: version.value,
        name_span,
        version_span,
        dev: pending.kind.is_development(),
        optional: false,
        registry: None,
        resolved_version: None,
        has_additional_version_constraints: false,
    })
}

fn newline_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        content
            .bytes()
            .enumerate()
            .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
    );
    starts
}

fn span_from_absolute(
    content: &str,
    newline_starts: &[usize],
    absolute_start: usize,
    absolute_end: usize,
) -> Option<Span> {
    let value = content.get(absolute_start..absolute_end)?;
    if value.is_empty() || value.bytes().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return None;
    }

    let line_index = newline_starts
        .partition_point(|line_start| *line_start <= absolute_start)
        .checked_sub(1)?;
    let line_absolute_start = *newline_starts.get(line_index)?;

    Some(Span {
        line: u32::try_from(line_index).ok()?,
        line_start: u32::try_from(absolute_start.checked_sub(line_absolute_start)?).ok()?,
        line_end: u32::try_from(absolute_end.checked_sub(line_absolute_start)?).ok()?,
    })
}

fn event_content_start(event_end: u64, event: &BytesStart<'_>, is_empty: bool) -> Option<usize> {
    let event_end = usize::try_from(event_end).ok()?;
    let event_raw: &[u8] = event.as_ref();
    let closing_delimiter_len = if is_empty { 2 } else { 1 };
    event_end
        .checked_sub(event_raw.len())?
        .checked_sub(closing_delimiter_len)
}

fn located_attribute(
    event: &BytesStart<'_>,
    event_start: usize,
    attribute_name: &[u8],
) -> Result<Option<LocatedValue>, ()> {
    let mut located = None;
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|_| ())?;
        if attribute.key.as_ref() != attribute_name {
            continue;
        }

        let (relative_start, relative_end) =
            raw_attribute_value_range(event, attribute_name).ok_or(())?;
        let absolute_start = event_start.checked_add(relative_start).ok_or(())?;
        let absolute_end = event_start.checked_add(relative_end).ok_or(())?;
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, event.decoder())
            .map_err(|_| ())?
            .into_owned();
        located = Some(LocatedValue {
            value,
            absolute_start,
            absolute_end,
        });
    }
    Ok(located)
}

fn raw_attribute_value_range(
    event: &BytesStart<'_>,
    attribute_name: &[u8],
) -> Option<(usize, usize)> {
    let raw: &[u8] = event.as_ref();
    let mut cursor = event.name().as_ref().len();

    while cursor < raw.len() {
        while raw.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if cursor == raw.len() {
            return None;
        }

        let name_start = cursor;
        while raw
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        {
            cursor += 1;
        }
        let name_end = cursor;

        while raw.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if raw.get(cursor) != Some(&b'=') {
            return None;
        }
        cursor += 1;
        while raw.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }

        let quote = *raw.get(cursor)?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        cursor += 1;
        let value_start = cursor;
        while raw.get(cursor) != Some(&quote) {
            cursor += 1;
            raw.get(cursor)?;
        }
        let value_end = cursor;
        cursor += 1;

        if raw.get(name_start..name_end)? == attribute_name {
            return Some((value_start, value_end));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span_text(content: &str, span: Span) -> &str {
        let line = content.lines().nth(span.line as usize).unwrap();
        &line[span.line_start as usize..span.line_end as usize]
    }

    fn assert_dependency(
        content: &str,
        dependency: &Dependency,
        name: &str,
        version: &str,
        dev: bool,
    ) {
        assert_eq!(dependency.name, name);
        assert_eq!(dependency.version, version);
        assert_eq!(span_text(content, dependency.name_span), name);
        assert_eq!(span_text(content, dependency.version_span), version);
        assert_eq!(dependency.dev, dev);
        assert!(!dependency.optional);
        assert_eq!(dependency.registry, None);
        assert_eq!(dependency.resolved_version, None);
        assert!(!dependency.has_additional_version_constraints);
    }

    #[test]
    fn test_parse_attribute_forms() {
        let cases = [
            (
                r#"<Project><PackageReference Include="Package" Version="1.2.3" /></Project>"#,
                false,
            ),
            (
                r#"<Project><PackageVersion Include="Package" Version="1.2.3" /></Project>"#,
                false,
            ),
            (
                r#"<Project><PackageVersion Update="Package" Version="1.2.3" /></Project>"#,
                false,
            ),
            (
                r#"<Project><GlobalPackageReference Include="Package" Version="1.2.3" /></Project>"#,
                true,
            ),
            (
                r#"<Project><PackageVersion Include = 'Package' Version = '1.2.3' /></Project>"#,
                false,
            ),
            (
                r#"<Project><PackageVersion Version="1.2.3" Include="Package" /></Project>"#,
                false,
            ),
        ];

        let parser = CsharpParser::new();
        for (content, dev) in cases {
            let dependencies = parser.parse(content);
            assert_eq!(dependencies.len(), 1, "failed to parse {content}");
            assert_dependency(content, &dependencies[0], "Package", "1.2.3", dev);
        }
    }

    #[test]
    fn test_parse_multiline_start_tag() {
        let content = r#"<Project>
  <PackageVersion
    Include="Package"
    Version="1.2.3" />
</Project>"#;

        let dependencies = CsharpParser::new().parse(content);

        assert_eq!(dependencies.len(), 1);
        assert_dependency(content, &dependencies[0], "Package", "1.2.3", false);
    }

    #[test]
    fn test_ignore_package_declaration_in_comment() {
        let content =
            r#"<Project><!-- <PackageReference Include="Package" Version="1.2.3" /> --></Project>"#;

        assert!(CsharpParser::new().parse(content).is_empty());
    }

    #[test]
    fn test_skip_empty_or_multiline_attribute_values() {
        for content in [
            r#"<Project><PackageReference Include="" Version="1.2.3" /></Project>"#,
            r#"<Project><PackageReference Include="Package" Version="" /></Project>"#,
            "<Project><PackageReference Include=\"Package\" Version=\"1.2\n.3\" /></Project>",
        ] {
            assert!(
                CsharpParser::new().parse(content).is_empty(),
                "unexpected dependency from {content}"
            );
        }
    }

    #[test]
    fn test_attribute_value_positions() {
        let content = r#"<PackageVersion Include="Package" Version="1.2.3" />"#;

        let dependencies = CsharpParser::new().parse(content);

        assert_eq!(dependencies.len(), 1);
        let dependency = &dependencies[0];
        assert_eq!(dependency.name_span.line, 0);
        assert_eq!(dependency.name_span.line_start, 25);
        assert_eq!(dependency.name_span.line_end, 32);
        assert_eq!(dependency.version_span.line, 0);
        assert_eq!(dependency.version_span.line_start, 43);
        assert_eq!(dependency.version_span.line_end, 48);
        assert_dependency(content, dependency, "Package", "1.2.3", false);
    }

    #[test]
    fn test_parse_self_closing() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="Serilog" Version="3.1.1" />
  </ItemGroup>
</Project>
"#;
        let parser = CsharpParser::new();
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 2);

        let newtonsoft = deps.iter().find(|d| d.name == "Newtonsoft.Json").unwrap();
        assert_eq!(newtonsoft.version, "13.0.3");

        let serilog = deps.iter().find(|d| d.name == "Serilog").unwrap();
        assert_eq!(serilog.version, "3.1.1");
    }

    #[test]
    fn test_parse_expanded_format() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Microsoft.Extensions.Logging" Version="8.0.0" />
  </ItemGroup>
</Project>
"#;
        let parser = CsharpParser::new();
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "Microsoft.Extensions.Logging");
        assert_eq!(deps[0].version, "8.0.0");
    }

    #[test]
    fn test_version_positions() {
        let content = r#"
<Project>
  <ItemGroup>
    <PackageReference Include="Serilog" Version="3.1.1" />
  </ItemGroup>
</Project>
"#;
        let parser = CsharpParser::new();
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 1);
        let dep = &deps[0];
        assert!(dep.version_span.line_start > dep.name_span.line_end);
    }

    #[test]
    fn test_skip_no_version() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" />
    <PackageReference Include="Serilog" Version="3.1.1" />
  </ItemGroup>
</Project>
"#;
        let parser = CsharpParser::new();
        let deps = parser.parse(content);

        // Should only find Serilog (Newtonsoft.Json has no version)
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "Serilog");
    }

    #[test]
    fn test_multiple_item_groups() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Package1" Version="1.0.0" />
  </ItemGroup>
  <ItemGroup Condition="'$(Configuration)'=='Debug'">
    <PackageReference Include="Package2" Version="2.0.0" />
  </ItemGroup>
</Project>
"#;
        let parser = CsharpParser::new();
        let deps = parser.parse(content);

        assert_eq!(deps.len(), 2);
    }
}
