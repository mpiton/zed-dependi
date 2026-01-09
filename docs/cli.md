---
title: CLI Usage
layout: default
nav_order: 7
description: "Command-line interface for CI/CD integration"
---

# CLI Usage
{: .no_toc }

Use dependi-lsp in your CI/CD pipelines for automated vulnerability scanning.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Overview

The `dependi-lsp` binary includes a standalone CLI scan command for integrating vulnerability scanning into your CI/CD pipelines.

## Scan Command

```bash
dependi-lsp scan --file <path> [options]
```

### Options

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--file <path>` | `-f` | required | Path to dependency file |
| `--output <format>` | `-o` | `summary` | Output format: `summary`, `json`, `markdown` |
| `--min-severity <level>` | `-m` | `low` | Minimum severity: `low`, `medium`, `high`, `critical` |
| `--fail-on-vulns` | | `true` | Exit with code 1 if vulnerabilities found |

### Supported Files

| Language | Files |
|----------|-------|
| Rust | `Cargo.toml` |
| Node.js | `package.json` |
| Python | `requirements.txt`, `pyproject.toml` |
| Go | `go.mod` |
| PHP | `composer.json` |
| Dart | `pubspec.yaml` |
| .NET | `*.csproj` |

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success - no vulnerabilities found (or `--fail-on-vulns=false`) |
| `1` | Failure - vulnerabilities found, file error, or network error |

## Output Formats

### Summary (Default)

```bash
dependi-lsp scan --file Cargo.toml
```

```
Vulnerability Scan Results for Cargo.toml

  ⚠ Critical: 0
  ▲ High:     1
  ● Medium:   2
  ○ Low:      0
  ─────────────
  Total:      3

⚠ 3 vulnerabilities found!
```

### JSON Output

```bash
dependi-lsp scan --file Cargo.toml --output json
```

```json
{
  "file": "Cargo.toml",
  "summary": {
    "total": 3,
    "critical": 0,
    "high": 1,
    "medium": 2,
    "low": 0
  },
  "vulnerabilities": [
    {
      "package": "tokio",
      "version": "1.35.0",
      "id": "RUSTSEC-2024-0001",
      "severity": "high",
      "description": "Race condition in tokio::time",
      "url": "https://rustsec.org/advisories/RUSTSEC-2024-0001"
    }
  ]
}
```

### Markdown Output

```bash
dependi-lsp scan --file Cargo.toml --output markdown
```

Generates a formatted report with severity tables and detailed vulnerability listings, suitable for PR comments or documentation.

## CI/CD Examples

### GitHub Actions

Create `.github/workflows/security-scan.yml`:

```yaml
name: Security Scan

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install dependi-lsp
        run: cargo install --git https://github.com/mpiton/zed-dependi --bin dependi-lsp

      - name: Scan dependencies
        run: dependi-lsp scan --file Cargo.toml --min-severity high

      - name: Generate report
        if: always()
        run: |
          dependi-lsp scan --file Cargo.toml --output markdown > security-report.md

      - name: Upload report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: security-report
          path: security-report.md
```

### GitLab CI

Add to `.gitlab-ci.yml`:

```yaml
security-scan:
  stage: test
  image: rust:latest
  script:
    - cargo install --git https://github.com/mpiton/zed-dependi --bin dependi-lsp
    - dependi-lsp scan --file Cargo.toml --min-severity high
  artifacts:
    when: always
    paths:
      - security-report.md
    reports:
      sast: security-report.json
  allow_failure: false
```

### Scanning Multiple Files

For monorepos with multiple dependency files:

```yaml
- name: Scan all dependency files
  run: |
    dependi-lsp scan --file Cargo.toml --min-severity high
    dependi-lsp scan --file frontend/package.json --min-severity high
    dependi-lsp scan --file backend/requirements.txt --min-severity high
```

### PR Comment with Results

```yaml
- name: Post scan results to PR
  if: github.event_name == 'pull_request'
  uses: actions/github-script@v7
  with:
    script: |
      const fs = require('fs');
      const report = fs.readFileSync('security-report.md', 'utf8');
      github.rest.issues.createComment({
        issue_number: context.issue.number,
        owner: context.repo.owner,
        repo: context.repo.repo,
        body: report
      });
```

## Best Practices

### Block on High/Critical Only

Use `--min-severity high` to fail builds only on serious vulnerabilities:

```bash
dependi-lsp scan --file Cargo.toml --min-severity high
```

### Generate Reports for Audit

Always generate reports for audit trails:

```bash
dependi-lsp scan --file Cargo.toml --output json > scan-results.json
dependi-lsp scan --file Cargo.toml --output markdown > scan-report.md
```

### Scheduled Scans

Run daily scans to catch newly disclosed vulnerabilities:

```yaml
on:
  schedule:
    - cron: '0 6 * * *'  # Daily at 6 AM
```

### Don't Fail on Low Severity

For informational low-severity issues:

```bash
dependi-lsp scan --file Cargo.toml --fail-on-vulns=false
```

### Scan Lock Files When Available

For more accurate results, scan after installing dependencies:

```bash
# npm
npm ci
dependi-lsp scan --file package.json

# Cargo
cargo fetch
dependi-lsp scan --file Cargo.toml
```

## Troubleshooting

### Command Not Found

Ensure dependi-lsp is in your PATH:

```bash
cargo install --git https://github.com/mpiton/zed-dependi --bin dependi-lsp
```

### Network Errors

The scan requires network access to:
- Package registries (crates.io, npm, etc.)
- OSV.dev API (`https://api.osv.dev`)

Ensure these are accessible in your CI environment.

### Timeout Issues

For large projects, scanning may take time. Consider:
- Running scans in parallel for different files
- Increasing CI timeout limits
- Using `--min-severity high` to reduce processing
