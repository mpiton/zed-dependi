---
title: Configuration
layout: default
nav_order: 3
description: "Configure Dependi settings in Zed Editor"
---

# Configuration
{: .no_toc }

Customize Dependi behavior through Zed settings.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Configuration Location

Configure Dependi in your Zed `settings.json`:

- **User settings**: `~/.config/zed/settings.json` (Linux) or `~/Library/Application Support/Zed/settings.json` (macOS)
- **Project settings**: `.zed/settings.json` in your project root

## Full Configuration Example

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "inlay_hints": {
          "enabled": true,
          "show_up_to_date": true
        },
        "diagnostics": {
          "enabled": true
        },
        "cache": {
          "ttl_secs": 3600
        },
        "security": {
          "enabled": true,
          "show_in_hints": true,
          "show_diagnostics": true,
          "min_severity": "low"
        },
        "ignore": ["internal-*", "test-pkg"]
      }
    }
  }
}
```

## Configuration Options Reference

### Inlay Hints

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `inlay_hints.enabled` | boolean | `true` | Enable/disable inlay hints |
| `inlay_hints.show_up_to_date` | boolean | `true` | Show hints for up-to-date packages |

**Example:**
```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "inlay_hints": {
          "enabled": true,
          "show_up_to_date": false
        }
      }
    }
  }
}
```

### Diagnostics

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `diagnostics.enabled` | boolean | `true` | Enable/disable diagnostics for outdated dependencies |

**Example:**
```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "diagnostics": {
          "enabled": true
        }
      }
    }
  }
}
```

### Cache

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `cache.ttl_secs` | number | `3600` | Cache time-to-live in seconds (1 hour default) |

**Example:**
```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "cache": {
          "ttl_secs": 7200
        }
      }
    }
  }
}
```

{: .note }
The cache is stored in your system's cache directory. Increasing TTL reduces network requests but may show stale data.

### Security (Vulnerability Scanning)

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `security.enabled` | boolean | `true` | Enable/disable vulnerability scanning |
| `security.show_in_hints` | boolean | `true` | Show vulnerability count in inlay hints |
| `security.show_diagnostics` | boolean | `true` | Show vulnerability diagnostics |
| `security.min_severity` | string | `"low"` | Minimum severity to report: `low`, `medium`, `high`, `critical` |

**Example:**
```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "security": {
          "enabled": true,
          "show_in_hints": true,
          "show_diagnostics": true,
          "min_severity": "high"
        }
      }
    }
  }
}
```

### Ignore Patterns

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `ignore` | string[] | `[]` | Package names or glob patterns to ignore |

**Example:**
```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "ignore": [
          "internal-*",
          "my-private-pkg",
          "@company/*"
        ]
      }
    }
  }
}
```

### Private Registries

Configure custom npm registries for private packages. See [Private Registries]({% link registries/private.md %}) for detailed setup.

| Option | Type | Description |
|--------|------|-------------|
| `registries.npm.url` | string | Base registry URL |
| `registries.npm.scoped` | object | Scoped registry configuration |

**Example:**
```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "registries": {
          "npm": {
            "url": "https://registry.npmjs.org",
            "scoped": {
              "company": {
                "url": "https://npm.company.com",
                "auth": {
                  "type": "env",
                  "variable": "COMPANY_NPM_TOKEN"
                }
              }
            }
          }
        }
      }
    }
  }
}
```

{: .warning }
Scope names should **not** include the `@` prefix. Use `"company"` not `"@company"`.

## Cache Locations

Dependi stores cached data in your system's cache directory:

| Platform | Location |
|----------|----------|
| Linux | `~/.cache/dependi/cache.db` |
| macOS | `~/Library/Caches/dependi/cache.db` |
| Windows | `%LOCALAPPDATA%\dependi\cache.db` |

### Clearing the Cache

To force refresh all package data:

```bash
# Linux
rm -rf ~/.cache/dependi/

# macOS
rm -rf ~/Library/Caches/dependi/

# Windows
rmdir /s %LOCALAPPDATA%\dependi
```

Then restart Zed. The cache will rebuild as you open dependency files.

## Configuration Tips

### Offline Work

For working offline, increase the cache TTL:

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "cache": {
          "ttl_secs": 86400
        }
      }
    }
  }
}
```

### Noisy Projects

For projects with many internal packages, use ignore patterns:

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "ignore": ["@internal/*", "dev-*"]
      }
    }
  }
}
```

### Security-Focused Setup

For strict security scanning:

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "security": {
          "enabled": true,
          "show_in_hints": true,
          "show_diagnostics": true,
          "min_severity": "medium"
        }
      }
    }
  }
}
```

### Minimal UI

For a cleaner interface showing only updates:

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "inlay_hints": {
          "enabled": true,
          "show_up_to_date": false
        }
      }
    }
  }
}
```

## Troubleshooting Configuration

### Configuration Not Applying

1. Verify JSON syntax is valid
2. Ensure settings are under the correct path:
   ```json
   {
     "lsp": {
       "dependi": {
         "initialization_options": {
           // settings here
         }
       }
     }
   }
   ```
3. Restart Zed after configuration changes
4. Check for typos in setting names

### Debugging

Run Zed with debug logging to see configuration loading:

```bash
RUST_LOG=debug zed --foreground
```
