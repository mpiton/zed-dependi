//! Parser for C# .csproj files (NuGet PackageReference format)

use super::{Dependency, Parser};

/// Parser for C# .csproj files
#[derive(Debug, Default)]
pub struct CsharpParser;

impl CsharpParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for CsharpParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let mut dependencies = Vec::new();

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Look for PackageReference elements
            // Format 1: <PackageReference Include="Package" Version="1.0.0" />
            // Format 2: <PackageReference Include="Package"><Version>1.0.0</Version></PackageReference>
            if trimmed.contains("<PackageReference") && trimmed.contains("Include=") {
                if let Some(dep) = parse_package_reference(line, line_num) {
                    dependencies.push(dep);
                }
            }
        }

        dependencies
    }
}

/// Parse a PackageReference XML element
fn parse_package_reference(line: &str, line_num: u32) -> Option<Dependency> {
    // Extract Include attribute (package name)
    let include_start = line.find("Include=\"")? + 9;
    let include_content = &line[include_start..];
    let include_end = include_content.find('"')?;
    let name = &include_content[..include_end];

    // Try to find Version attribute on same line
    let version = if let Some(version_attr_start) = line.find("Version=\"") {
        let version_content = &line[version_attr_start + 9..];
        let version_end = version_content.find('"')?;
        version_content[..version_end].to_string()
    } else if let Some(version_elem_start) = line.find("<Version>") {
        // Format: <Version>1.0.0</Version>
        let version_content = &line[version_elem_start + 9..];
        let version_end = version_content.find('<')?;
        version_content[..version_end].to_string()
    } else {
        // Version might be centrally managed (Directory.Packages.props)
        // Skip for now
        return None;
    };

    // Calculate positions
    let name_pattern = format!("\"{}\"", name);
    let name_pos = line.find(&name_pattern)?;
    let name_start = (name_pos + 1) as u32;
    let name_end = name_start + name.len() as u32;

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
        dev: false, // NuGet doesn't have explicit dev dependencies in .csproj
        optional: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(dep.version_start > dep.name_end);
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
