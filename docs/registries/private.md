---
title: Private Registries
layout: default
parent: Registries
nav_order: 1
description: "Configure private package registries for enterprise environments"
---

# Private Registries
{: .no_toc }

Configure Dependi to work with private package registries for enterprise environments.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Overview

Dependi supports custom registry configuration for organizations that need to:
- Host internal packages privately
- Use self-hosted registry solutions (Verdaccio, Artifactory, etc.)
- Comply with security requirements by proxying public registries
- Mix public and private packages in the same project

## Supported Ecosystems

| Ecosystem | Custom Registry | Scoped Registries | Authentication |
|-----------|-----------------|-------------------|----------------|
| npm | Yes | Yes | Environment Variables |
| Cargo | Planned | Planned | - |
| PyPI | Planned | - | - |
| Other | Not yet | - | - |

## npm Configuration

npm has full support for custom registries, including scoped package routing.

### Single Registry (All Packages)

Route all npm packages through a private registry:

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "registries": {
          "npm": {
            "url": "https://npm.company.com"
          }
        }
      }
    }
  }
}
```

All packages (`express`, `lodash`, etc.) will be fetched from `https://npm.company.com`.

### Scoped Registries (Public + Private Mix)

Use different registries for different package scopes:

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
                "url": "https://npm.company.com"
              },
              "internal": {
                "url": "https://npm.company.com"
              }
            }
          }
        }
      }
    }
  }
}
```

This routes:
- `express` → `https://registry.npmjs.org/express` (public)
- `@company/utils` → `https://npm.company.com/@company/utils` (private)
- `@internal/logger` → `https://npm.company.com/@internal/logger` (private)

{: .warning }
Scope names in configuration should **not** include the `@` prefix. Use `"company"` not `"@company"`.

### Common Private Registry Solutions

| Registry | Use Case | Example URL |
|----------|----------|-------------|
| **Verdaccio** | Local development, small teams | `http://localhost:4873` |
| **Artifactory** | Enterprise artifact management | `https://artifactory.company.com/api/npm/npm-local` |
| **npm Enterprise** | Scalable private npm | `https://npm.company.com` |
| **GitHub Packages** | GitHub-integrated CI/CD | `https://npm.pkg.github.com` |
| **GitLab Packages** | GitLab-integrated CI/CD | `https://gitlab.company.com/api/v4/packages/npm/` |
| **AWS CodeArtifact** | AWS-native artifact management | `https://domain-123456789012.d.codeartifact.region.amazonaws.com/npm/repo/` |

## Authentication

Dependi reads authentication tokens from **environment variables only**. Tokens are never stored in configuration files.

### Setting Up Authentication

1. **Set the environment variable** before starting Zed:

```bash
# npm private registry
export COMPANY_NPM_TOKEN="npm_xxxxxxxxxxxxxxxxxxxxxxxxxx"

# GitHub Packages
export GITHUB_TOKEN="ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
```

2. **Configure authentication in Zed settings**:

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
              },
              "github": {
                "url": "https://npm.pkg.github.com",
                "auth": {
                  "type": "env",
                  "variable": "GITHUB_TOKEN"
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

### Authentication Type

Currently, only the `env` authentication type is supported:

| Type | Description |
|------|-------------|
| `env` | Read token from environment variable |

The token is sent as a Bearer token in the `Authorization` header for HTTPS requests only.

### Security Best Practices

1. **Never hardcode tokens** in configuration files
2. **Use environment variables** for all tokens
3. **HTTPS only** - authentication headers are only sent over HTTPS
4. **Least privilege** - use read-only tokens when possible
5. **Rotate tokens regularly** - regenerate tokens periodically
6. **Use secret managers** in CI/CD:

```bash
# AWS Secrets Manager
export COMPANY_NPM_TOKEN=$(aws secretsmanager get-secret-value \
  --secret-id npm/token --query SecretString --output text)

# HashiCorp Vault
export COMPANY_NPM_TOKEN=$(vault kv get -field=token secret/npm)

# GitHub Actions - use ${{ secrets.NPM_TOKEN }} in workflow
```

### Token Rotation

When rotating tokens:

1. Generate new token in registry
2. Update environment variable
3. Restart Zed to apply changes

```bash
export COMPANY_NPM_TOKEN="npm_new_token_here"
# Restart Zed or reload window
```

## Complete Configuration Example

Full configuration for an organization using multiple registries:

```json
{
  "lsp": {
    "dependi": {
      "initialization_options": {
        "registries": {
          "npm": {
            "url": "https://registry.npmjs.org",
            "scoped": {
              "acme": {
                "url": "https://npm.acme-corp.com",
                "auth": {
                  "type": "env",
                  "variable": "ACME_NPM_TOKEN"
                }
              },
              "acme-internal": {
                "url": "https://npm.acme-corp.com",
                "auth": {
                  "type": "env",
                  "variable": "ACME_NPM_TOKEN"
                }
              },
              "github": {
                "url": "https://npm.pkg.github.com",
                "auth": {
                  "type": "env",
                  "variable": "GITHUB_TOKEN"
                }
              }
            }
          }
        },
        "inlay_hints": {
          "enabled": true
        },
        "security": {
          "enabled": true
        }
      }
    }
  }
}
```

With environment:

```bash
export ACME_NPM_TOKEN="npm_xxxxxxxxxxxxx"
export GITHUB_TOKEN="ghp_xxxxxxxxxxxxx"
```

## Troubleshooting

### 401 Unauthorized

**Symptoms**: Package info not loading, error in logs

**Solutions**:
1. Verify environment variable is set: `echo $COMPANY_NPM_TOKEN`
2. Check token has read permissions on the registry
3. Ensure token hasn't expired
4. Verify the variable name in config matches exactly

### 404 Package Not Found

**Symptoms**: `? Unknown` hint for private packages

**Solutions**:
1. Verify package name and scope spelling
2. Check registry URL is correct
3. Ensure the package exists in the private registry
4. Verify scope is configured (without `@` prefix)

### Connection Timeout

**Symptoms**: Slow or failed package lookups

**Solutions**:
1. Check network connectivity to registry
2. Verify firewall allows HTTPS to registry URL
3. Check if VPN is required for internal registries
4. Verify registry URL is accessible in browser

### Configuration Not Applied

**Symptoms**: Still using public registry despite configuration

**Solutions**:
1. Verify JSON syntax is valid
2. Check settings path: `lsp.dependi.initialization_options.registries`
3. Restart Zed after configuration changes
4. Check for typos in scope names

### Debug Logging

Enable debug logging to troubleshoot registry issues:

```bash
RUST_LOG=debug zed --foreground
```

Look for log entries like:
```
[DEBUG] Querying registry: https://npm.company.com/@company/utils
[DEBUG] Using auth header: Bearer npm_... (redacted)
[DEBUG] Response status: 200 OK
```

## Future Enhancements

### Cargo Custom Registries (Planned)

```json
{
  "registries": {
    "cargo": {
      "url": "https://crates.my-org.com/api/v1",
      "alternative": {
        "my-registry": {
          "url": "https://crates.my-org.com/api/v1",
          "index": "https://github.com/my-org/crates-index"
        }
      }
    }
  }
}
```

### PyPI Custom Registries (Planned)

```json
{
  "registries": {
    "pypi": {
      "url": "https://pypi.company.com/simple",
      "auth": {
        "type": "env",
        "variable": "PYPI_TOKEN"
      }
    }
  }
}
```

### Credential File Support (Planned)

Support for reading tokens from:
- `.npmrc` files
- `~/.cargo/credentials.toml`

## References

- [npm Registry API](https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md)
- [npm Scoped Packages](https://docs.npmjs.com/cli/v6/using-npm/scope)
- [Verdaccio Documentation](https://verdaccio.org/docs/what-is-verdaccio)
- [Artifactory npm Registry](https://jfrog.com/help/r/jfrog-artifactory-documentation/npm-registry)
- [GitHub Packages npm Registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-npm-registry)
- [AWS CodeArtifact](https://docs.aws.amazon.com/codeartifact/latest/ug/npm-auth.html)
