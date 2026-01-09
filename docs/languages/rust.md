---
title: Rust
layout: default
parent: Languages
nav_order: 1
description: "Rust/Cargo.toml support"
---

# Rust
{: .no_toc }

Support for Rust projects using Cargo.toml.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Supported Files

| File | Description |
|------|-------------|
| `Cargo.toml` | Main dependency manifest |

## Registry

**crates.io** - The official Rust package registry

- Base URL: `https://crates.io/api/v1`
- Rate limit: 1 request per second (strictly enforced)
- Documentation: [crates.io](https://crates.io)

## Dependency Formats

Dependi parses all standard Cargo dependency formats:

### Simple Version

```toml
[dependencies]
serde = "1.0"
```

### Version with Features

```toml
[dependencies]
tokio = { version = "1.0", features = ["full"] }
```

### Inline Table

```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
```

### Development Dependencies

```toml
[dev-dependencies]
criterion = "0.5"
```

### Build Dependencies

```toml
[build-dependencies]
cc = "1.0"
```

## Version Specification

Cargo uses semantic versioning with these operators:

| Syntax | Meaning |
|--------|---------|
| `"1.0.0"` | Exactly 1.0.0 |
| `"^1.0"` | >=1.0.0, <2.0.0 (default) |
| `"~1.0"` | >=1.0.0, <1.1.0 |
| `"*"` | Any version |
| `">=1.0"` | 1.0.0 or higher |
| `">=1.0, <2.0"` | Range |

## Special Cases

### Workspace Dependencies

```toml
[workspace.dependencies]
serde = "1.0"

[dependencies]
serde = { workspace = true }
```

Dependi tracks workspace dependencies in the root `Cargo.toml`.

### Path Dependencies

```toml
[dependencies]
my-crate = { path = "../my-crate" }
```

Path dependencies show `→ Local` hint (no registry lookup).

### Git Dependencies

```toml
[dependencies]
my-crate = { git = "https://github.com/user/repo" }
```

Git dependencies show `→ Git` hint (no registry lookup).

### Yanked Versions

Yanked versions on crates.io show `⊘ Yanked` hint. These versions should be updated immediately.

## Vulnerability Database

Rust vulnerabilities are sourced from:
- [RustSec Advisory Database](https://rustsec.org/)
- GitHub Security Advisories

## Example Cargo.toml

```toml
[package]
name = "my-project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }  # ✓
tokio = { version = "1.35", features = ["full"] }   # -> 1.36.0
anyhow = "1.0"                                       # ✓
ring = "0.16"                                        # ⚠ 2 vulns

[dev-dependencies]
criterion = "0.5"                                    # ✓
```

## Troubleshooting

### Rate Limiting

crates.io has strict rate limits (1 req/s). If you see intermittent failures:
- Wait for the rate limiter to reset
- Dependi automatically handles rate limiting with delays
- Large `Cargo.toml` files may take longer on first load

### Name Normalization

Crate names are normalized: `foo-bar` and `foo_bar` are equivalent. Dependi handles this automatically.

### Workspace Resolution

For workspace projects, open the root `Cargo.toml` to see all dependencies. Individual member `Cargo.toml` files inherit workspace versions.
