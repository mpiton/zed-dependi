# Registry API Documentation

This document provides a comprehensive overview of all supported package registries
in dependi-lsp, including API details, rate limits, and common operations.

## Quick Reference

| Registry | Ecosystem | Dependency File | Base URL | Rate Limit |
|----------|-----------|-----------------|----------|------------|
| [crates.io](#cratesio) | Rust | `Cargo.toml` | `https://crates.io/api/v1` | 1 req/s |
| [npm](#npm) | Node.js | `package.json` | `https://registry.npmjs.org` | ~100/min |
| [PyPI](#pypi) | Python | `requirements.txt`, `pyproject.toml` | `https://pypi.org/pypi` | ~20 req/s |
| [Go Proxy](#go-proxy) | Go | `go.mod` | `https://proxy.golang.org` | Fair use |
| [Packagist](#packagist) | PHP | `composer.json` | `https://repo.packagist.org` | ~60/min |
| [pub.dev](#pubdev) | Dart/Flutter | `pubspec.yaml` | `https://pub.dev/api` | ~100/min |
| [NuGet](#nuget) | .NET | `*.csproj` | `https://api.nuget.org/v3` | Fair use |
| [RubyGems](#rubygems) | Ruby | `Gemfile` | `https://rubygems.org/api/v1` | ~10 req/s |

## Common Data Model

All registries return a unified `VersionInfo` structure:

```rust
pub struct VersionInfo {
    pub latest: Option<String>,           // Latest stable version
    pub latest_prerelease: Option<String>, // Latest prerelease
    pub versions: Vec<String>,            // All available versions
    pub description: Option<String>,      // Package description
    pub homepage: Option<String>,         // Homepage URL
    pub repository: Option<String>,       // Repository URL
    pub license: Option<String>,          // SPDX license identifier
    pub vulnerabilities: Vec<Vulnerability>, // Known vulnerabilities (via OSV)
    pub deprecated: bool,                 // Deprecation status
    pub yanked: bool,                     // Whether latest is yanked
    pub yanked_versions: Vec<String>,     // List of yanked versions
    pub release_dates: HashMap<String, DateTime<Utc>>, // Version timestamps
}
```

## Registry Details

### crates.io

**Ecosystem:** Rust
**Dependency File:** `Cargo.toml`

#### API Endpoint

```
GET https://crates.io/api/v1/crates/{crate_name}
```

#### Rate Limiting

- **Limit:** 1 request per second (strictly enforced)
- **Headers:** `X-RateLimit-Remaining`, `X-RateLimit-Reset`
- **Enforcement:** Client-side rate limiter included

#### Quirks

- Name normalization: `foo-bar` and `foo_bar` are equivalent
- `yanked` field indicates withdrawn versions
- `max_stable_version` provides latest stable directly

#### External Links

- [API Documentation](https://crates.io/data-access)
- [Rate Limiting Policy](https://crates.io/policies#crawlers)

---

### npm

**Ecosystem:** Node.js / JavaScript
**Dependency File:** `package.json`

#### API Endpoint

```
GET https://registry.npmjs.org/{package-name}
GET https://registry.npmjs.org/@scope%2fpackage-name  # Scoped packages
```

#### Rate Limiting

- **Limit:** No hard limit, but ~100/min recommended
- **Enforcement:** IP-based blocking for abuse

#### Quirks

- Scoped packages require URL encoding (`/` → `%2f`)
- `dist-tags.latest` and `dist-tags.next` for version channels
- `deprecated` field is a string message when present
- Repository URL may have `git+https://` or `.git` suffix

#### External Links

- [Registry API](https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md)
- [Package Metadata](https://github.com/npm/registry/blob/main/docs/responses/package-metadata.md)

---

### PyPI

**Ecosystem:** Python
**Dependency Files:** `requirements.txt`, `pyproject.toml`

#### API Endpoint

```
GET https://pypi.org/pypi/{package-name}/json
```

#### Rate Limiting

- **Limit:** ~20 requests per second
- **CDN:** Fastly CDN caching

#### Quirks

- Name normalization per PEP 503: `Flask` = `flask`, `typing_extensions` = `typing-extensions`
- `Development Status :: 7 - Inactive` classifier indicates deprecation
- Version format follows PEP 440 (not strict semver)
- Date format: ISO 8601 without timezone

#### External Links

- [JSON API](https://warehouse.pypa.io/api-reference/json.html)
- [PEP 503](https://peps.python.org/pep-0503/)
- [PEP 440](https://peps.python.org/pep-0440/)

---

### Go Proxy

**Ecosystem:** Go
**Dependency File:** `go.mod`

#### API Endpoints

```
GET https://proxy.golang.org/{module}/@v/list     # List versions
GET https://proxy.golang.org/{module}/@latest     # Latest version
GET https://proxy.golang.org/{module}/@v/{v}.info # Version info
```

#### Rate Limiting

- **Limit:** Fair use policy, no hard limit
- **CDN:** Heavy caching

#### Quirks

- Module path encoding: uppercase → `!` + lowercase (`Azure` → `!azure`)
- Version prefix: `v` required (`v1.0.0`, not `1.0.0`)
- Pseudo-versions for untagged commits
- Major version suffix in path for v2+ (`module/v2`)

#### External Links

- [Module Proxy Protocol](https://go.dev/ref/mod#module-proxy)
- [Version Numbering](https://go.dev/ref/mod#versions)

---

### Packagist

**Ecosystem:** PHP / Composer
**Dependency File:** `composer.json`

#### API Endpoint

```
GET https://repo.packagist.org/p2/{vendor}/{package}.json
```

#### Rate Limiting

- **Limit:** ~60 requests per minute
- **CDN:** Heavy caching

#### Quirks

- Package name format: `vendor/package` (required)
- Dev versions: `dev-master`, `dev-main`, `1.0.x-dev` (filtered out)
- `abandoned` field can be `true` or replacement package name

#### External Links

- [Packagist API](https://packagist.org/apidoc)
- [Composer Versions](https://getcomposer.org/doc/articles/versions.md)

---

### pub.dev

**Ecosystem:** Dart / Flutter
**Dependency File:** `pubspec.yaml`

#### API Endpoint

```
GET https://pub.dev/api/packages/{package-name}
```

#### Rate Limiting

- **Limit:** ~100 requests per minute
- **CDN:** Edge caching

#### Quirks

- `retracted` field equivalent to yanked
- `discontinued` in pubspec indicates deprecation
- SDK constraints in `environment` field

#### External Links

- [pub.dev API](https://pub.dev/help/api)
- [Pubspec Format](https://dart.dev/tools/pub/pubspec)

---

### NuGet

**Ecosystem:** .NET
**Dependency File:** `*.csproj`

#### API Endpoint

```
GET https://api.nuget.org/v3/registration5-semver1/{package-id}/index.json
```

#### Rate Limiting

- **Limit:** Fair use policy
- **CDN:** Azure CDN caching

#### Quirks

- Package ID is case-insensitive but URLs use lowercase
- Paged responses for packages with many versions
- `listed: false` hides from search (still downloadable)
- `deprecation` object present when deprecated

#### External Links

- [NuGet Server API](https://learn.microsoft.com/en-us/nuget/api/overview)
- [Package Metadata](https://learn.microsoft.com/en-us/nuget/api/registration-base-url-resource)

---

### RubyGems

**Ecosystem:** Ruby
**Dependency File:** `Gemfile`

#### API Endpoints

```
GET https://rubygems.org/api/v1/gems/{gem-name}.json      # Gem info
GET https://rubygems.org/api/v1/versions/{gem-name}.json  # All versions
```

#### Rate Limiting

- **Limit:** ~10 requests per second
- **Enforcement:** Blocking for abuse

#### Quirks

- Prerelease format: `.pre.1` (not `-pre.1`)
- No deprecation flag in API
- Platform gems may have suffix (`-java`, `-x86_64-linux`)

#### External Links

- [RubyGems API](https://guides.rubygems.org/rubygems-org-api/)
- [Gem Specification](https://guides.rubygems.org/specification-reference/)

---

## Vulnerability Detection

Vulnerability information is **not** fetched from package registries. Instead,
dependi-lsp uses the [OSV (Open Source Vulnerabilities)](https://osv.dev/) API
to check for known security issues across all ecosystems.

## Troubleshooting

### Rate Limit Errors

If you encounter 429 (Too Many Requests) errors:

1. **crates.io**: Wait 1 second between requests (automatic)
2. **Other registries**: Reduce request frequency or implement backoff

### Package Not Found

- Check package name spelling and case sensitivity
- For scoped npm packages, ensure correct `@scope/name` format
- For Packagist, use `vendor/package` format

### Timeout Errors

Default timeout is 10 seconds. Network issues or slow registries may cause timeouts.
Consider implementing retry logic for production use.
