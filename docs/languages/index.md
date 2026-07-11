---
title: Languages
layout: default
nav_order: 5
has_children: true
description: "Supported languages and ecosystems"
---

# Supported Languages
{: .no_toc }

Dependi supports 8 programming languages and their package ecosystems.
{: .fs-6 .fw-300 }

---

## Overview

| Language | Dependency File | Registry | Status |
|----------|-----------------|----------|--------|
| [Rust]({% link languages/rust.md %}) | `Cargo.toml` | crates.io | Full support |
| [JavaScript/TypeScript]({% link languages/nodejs.md %}) | `package.json` | npm | Full support |
| [Python]({% link languages/python.md %}) | `requirements.txt`, `pyproject.toml` | PyPI | Full support |
| [Go]({% link languages/go.md %}) | `go.mod` | proxy.golang.org | Full support |
| [PHP]({% link languages/php.md %}) | `composer.json` | Packagist | Full support |
| [Dart/Flutter]({% link languages/dart.md %}) | `pubspec.yaml` | pub.dev | Full support |
| [C#/.NET]({% link languages/dotnet.md %}) | `*.csproj`, `Directory.Build.props`, `Directory.Packages.props` | NuGet | Full support |
| [Ruby]({% link languages/ruby.md %}) | `Gemfile` | RubyGems.org | Full support |

## Features by Language

All languages support:
- Inlay hints showing latest versions
- Diagnostics for outdated dependencies
- Code actions to update versions
- Vulnerability scanning via OSV.dev
- Hover information with package details

## File Detection

Dependi automatically detects dependency files by name:

```
Cargo.toml        → Rust
package.json      → Node.js
requirements.txt  → Python
pyproject.toml    → Python
go.mod            → Go
composer.json     → PHP
pubspec.yaml      → Dart
*.csproj                    → .NET
Directory.Build.props       → .NET
Directory.Packages.props    → .NET
Gemfile           → Ruby
```

## Version Formats

Each ecosystem has its own version specification format:

| Ecosystem | Example | Meaning |
|-----------|---------|---------|
| Rust | `"1.0"`, `"^1.0"`, `"~1.0"` | Cargo semver |
| npm | `"^1.0.0"`, `"~1.0.0"`, `">=1.0"` | npm semver |
| Python | `==1.0.0`, `>=1.0,<2.0` | PEP 440 |
| Go | `v1.0.0`, `v1.0.0+incompatible` | Go modules |
| PHP | `^1.0`, `~1.0`, `>=1.0 <2.0` | Composer |
| Dart | `^1.0.0`, `">=1.0.0 <2.0.0"` | pub |
| .NET | `1.0.0`, `[1.0,2.0)` | NuGet |
| Ruby | `~> 1.0`, `>= 1.0, < 2.0` | Bundler |

Dependi understands these formats and extracts the correct version for checking.

## Adding Language Support

Want to add support for a new language? See the [Contributing Guide](https://github.com/mpiton/zed-dependi/blob/main/CONTRIBUTING.md#adding-support-for-new-languages) for instructions on:

1. Creating a parser
2. Implementing a registry client
3. Registering the language
4. Adding tests
