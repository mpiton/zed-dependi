---
title: C#/.NET
layout: default
parent: Languages
nav_order: 7
description: ".NET project file (csproj) support"
---

# C#/.NET
{: .no_toc }

Support for .NET projects using *.csproj files.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Supported Files

| File | Description |
|------|-------------|
| `*.csproj` | C# project file |

## Registry

**NuGet** - The .NET package manager

- Base URL: `https://api.nuget.org/v3`
- Rate limit: Fair use policy
- Documentation: [nuget.org](https://www.nuget.org)

## Dependency Format

### PackageReference (SDK-style)

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="Serilog" Version="3.1.1" />
  </ItemGroup>
</Project>
```

### With Attributes

```xml
<PackageReference Include="xunit" Version="2.6.0" />
<PackageReference Include="xunit.runner.visualstudio" Version="2.5.0">
  <PrivateAssets>all</PrivateAssets>
  <IncludeAssets>runtime; build; native</IncludeAssets>
</PackageReference>
```

### Central Package Management

```xml
<!-- Directory.Packages.props -->
<Project>
  <PropertyGroup>
    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>
  </PropertyGroup>
  <ItemGroup>
    <PackageVersion Include="Newtonsoft.Json" Version="13.0.3" />
  </ItemGroup>
</Project>
```

## Version Specification

NuGet supports various version formats:

| Syntax | Meaning |
|--------|---------|
| `1.0.0` | Minimum version |
| `[1.0.0]` | Exactly 1.0.0 |
| `[1.0.0,2.0.0]` | Range inclusive |
| `[1.0.0,2.0.0)` | Range exclusive upper |
| `(,1.0.0]` | Maximum version |

Most projects use simple version (`1.0.0`) which means "minimum version".

## Special Cases

### Framework References

```xml
<FrameworkReference Include="Microsoft.AspNetCore.App" />
```

Framework references show `→ Framework` hint.

### Project References

```xml
<ProjectReference Include="..\MyLib\MyLib.csproj" />
```

Project references show `→ Project` hint.

### Unlisted Packages

Packages marked as unlisted on NuGet show `⚠ Unlisted` hint. They're still downloadable but hidden from search.

### Deprecated Packages

Deprecated packages show `⚠ Deprecated` with the deprecation reason on hover.

### Package IDs

NuGet package IDs are case-insensitive but URLs use lowercase. Dependi handles this automatically.

## Vulnerability Database

.NET vulnerabilities are sourced from:
- [GitHub Advisory Database](https://github.com/advisories)
- NuGet vulnerability metadata

## Example .csproj

```xml
<Project Sdk="Microsoft.NET.Sdk">

  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>

  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />        <!-- ✓ -->
    <PackageReference Include="Serilog" Version="3.0.0" />                  <!-- -> 3.1.1 -->
    <PackageReference Include="Dapper" Version="2.1.0" />                   <!-- ✓ -->
    <PackageReference Include="Polly" Version="8.2.0" />                    <!-- ✓ -->
  </ItemGroup>

  <ItemGroup>
    <PackageReference Include="xunit" Version="2.6.0" />                    <!-- ✓ -->
    <PackageReference Include="Moq" Version="4.20.0" />                     <!-- -> 4.20.70 -->
  </ItemGroup>

</Project>
```

## Tooling Integration

After updating `.csproj` with Dependi:

```bash
# Restore packages
dotnet restore

# Update specific package
dotnet add package Newtonsoft.Json --version 13.0.3

# List outdated packages
dotnet list package --outdated

# Check for vulnerabilities
dotnet list package --vulnerable
```

## Troubleshooting

### Package Not Found

1. Verify package ID spelling (case-insensitive)
2. Check if package exists on nuget.org
3. For private feeds, configure `NuGet.config`

### Version Not Updating

1. Check for `Directory.Build.props` overrides
2. Review Central Package Management settings
3. Clear NuGet cache: `dotnet nuget locals all --clear`

### Multiple Target Frameworks

For multi-targeting projects:
```xml
<TargetFrameworks>net6.0;net7.0;net8.0</TargetFrameworks>
```

Dependi shows the latest version compatible with any target framework.

### Private NuGet Feeds

For private feeds:
1. Configure in `NuGet.config`
2. Set up authentication
3. Note: Dependi currently uses nuget.org only
