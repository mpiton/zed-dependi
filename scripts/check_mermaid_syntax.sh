#!/usr/bin/env bash
# T2 — Validate every fenced ```mermaid block in a markdown doc parses with mermaid-cli.
# Usage: scripts/check_mermaid_syntax.sh [path/to/doc.md]
# Exit: 0 if all blocks valid (or doc has no mermaid blocks), 1 otherwise.

set -euo pipefail

DOC="${1:-docs/architecture.md}"
if [ ! -f "$DOC" ]; then
    echo "ERROR: $DOC not found" >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if ! command -v mmdc >/dev/null 2>&1; then
    echo "Installing mermaid-cli into $TMP (one-shot)…"
    npm install --no-save --prefix "$TMP" @mermaid-js/mermaid-cli >/dev/null 2>&1 \
        || { echo "ERROR: failed to install @mermaid-js/mermaid-cli" >&2; exit 1; }
    MMDC="$TMP/node_modules/.bin/mmdc"
else
    MMDC="mmdc"
fi

awk -v outdir="$TMP" '
    /^[[:space:]]*```mermaid[[:space:]]*$/ { in_block=1; n++; out=sprintf("%s/block_%02d.mmd", outdir, n); next }
    /^[[:space:]]*```[[:space:]]*$/ && in_block { in_block=0; close(out); next }
    in_block { print > out }
' "$DOC"

shopt -s nullglob
blocks=("$TMP"/block_*.mmd)
if [ ${#blocks[@]} -eq 0 ]; then
    echo "No mermaid blocks found in $DOC."
    exit 0
fi

# Puppeteer needs --no-sandbox under CI containers (it cannot drop privileges
# when the runner already executes as a non-root user inside an unprivileged
# container). The same config is harmless on dev machines.
PUPPETEER_CFG="$TMP/puppeteer.json"
cat > "$PUPPETEER_CFG" <<'JSON'
{ "args": ["--no-sandbox", "--disable-setuid-sandbox"] }
JSON

failures=0
for f in "${blocks[@]}"; do
    err_log="$TMP/$(basename "$f").err"
    if ! "$MMDC" -i "$f" -o "$TMP/$(basename "$f").svg" -p "$PUPPETEER_CFG" --quiet >"$err_log" 2>&1; then
        echo "FAIL: $(basename "$f")" >&2
        head -10 "$f" >&2
        echo "--- mmdc stderr ---" >&2
        head -20 "$err_log" >&2
        echo "---" >&2
        failures=$((failures + 1))
    fi
done

if [ "$failures" -gt 0 ]; then
    echo "$failures mermaid block(s) failed validation in $DOC" >&2
    exit 1
fi
echo "All ${#blocks[@]} mermaid block(s) valid in $DOC."
