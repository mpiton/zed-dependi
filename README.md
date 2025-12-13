# Dependi for Zed

Dependency management extension for the [Zed](https://zed.dev) editor.

**Version:** 0.1.0 (MVP)

## Features

- **Inlay Hints**: See latest versions inline next to your dependencies
  - `✓` - Version is up to date
  - `⬆ X.Y.Z` - Update available
  - `?` - Could not fetch version info
- **Hover Info**: Package descriptions, licenses, and links
- **Multi-language support**: Rust (Cargo.toml) and JavaScript/TypeScript (package.json)

## Supported Languages

| Language | File | Registry | Status |
|----------|------|----------|--------|
| Rust | `Cargo.toml` | crates.io | ✅ |
| JavaScript/TypeScript | `package.json` | npm | ✅ |
| Python | `requirements.txt`, `pyproject.toml` | PyPI | Planned |
| Go | `go.mod` | proxy.golang.org | Planned |

## Installation

### From Zed Extensions (Coming Soon)

Once published, you'll be able to install directly from Zed's extension marketplace.

### Manual Installation (Development)

1. Clone this repository
2. Build the LSP and extension:

```bash
# Build the LSP
cd dependi-lsp
cargo build --release

# Build the extension
cd ../dependi-zed
cargo build --release --target wasm32-wasip1
```

3. In Zed, run `zed: install dev extension` and select the `dependi-zed` directory

## Project Structure

```
zed-dependi/
├── dependi-lsp/           # Language Server (Rust binary)
│   ├── src/
│   │   ├── main.rs        # Entry point
│   │   ├── lib.rs         # Library exports
│   │   ├── backend.rs     # LSP implementation
│   │   ├── parsers/       # Dependency file parsers
│   │   │   ├── cargo.rs   # Cargo.toml parser
│   │   │   └── npm.rs     # package.json parser
│   │   ├── registries/    # Package registry clients
│   │   │   ├── crates_io.rs
│   │   │   └── npm.rs
│   │   ├── providers/     # LSP feature providers
│   │   │   └── inlay_hints.rs
│   │   └── cache/         # Caching layer
│   └── tests/             # Integration tests
├── dependi-zed/           # Zed Extension (WASM)
│   ├── extension.toml
│   └── src/lib.rs
├── PRD.md                 # Product Requirements
└── TASKS.md               # Task tracking
```

## Development

### Prerequisites

- Rust 1.75+ (tested with 1.91.1)
- `wasm32-wasip1` target: `rustup target add wasm32-wasip1`

### Building

```bash
# Build LSP (release)
cd dependi-lsp
cargo build --release

# Run tests
cargo test

# Build extension
cd ../dependi-zed
cargo build --release --target wasm32-wasip1
```

### Testing

```bash
# Run all tests (29 tests)
cd dependi-lsp
cargo test

# Run specific test modules
cargo test parsers::cargo
cargo test parsers::npm
cargo test registries
cargo test providers
```

### Debugging

```bash
# Run LSP with debug logs
cd dependi-lsp
RUST_LOG=debug cargo run

# View Zed logs
zed --foreground
```

## Configuration

Configure Dependi in your Zed `settings.json`:

```json
{
  "lsp": {
    "dependi": {
      "settings": {
        "inlayHints": {
          "enabled": true
        }
      }
    }
  }
}
```

## How It Works

1. When you open a dependency file (`Cargo.toml` or `package.json`), the LSP parses it to extract dependencies
2. For each dependency, it queries the appropriate registry (crates.io or npm)
3. Version information is cached to avoid repeated network requests
4. Inlay hints show whether each dependency is up-to-date or has updates available
5. Hovering over a dependency shows detailed package information

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Zed Editor                          │
├─────────────────────────────────────────────────────────────┤
│                    dependi-zed (WASM)                       │
│  - Downloads and launches the LSP binary                    │
└─────────────────────────────────────────────────────────────┘
                              │ stdio (JSON-RPC)
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   dependi-lsp (Binary)                      │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │   Parsers    │  │  Providers   │  │  Registries  │      │
│  ├──────────────┤  ├──────────────┤  ├──────────────┤      │
│  │ • Cargo.toml │  │ • Inlay Hints│  │ • crates.io  │      │
│  │ • package.json│ │ • Hover      │  │ • npm        │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                    Cache Layer                        │  │
│  │  • In-memory cache with TTL (1 hour)                 │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Roadmap

- [x] **v0.1.0 (MVP)**: Cargo.toml + package.json support with inlay hints
- [ ] **v0.2.0**: Diagnostics, code actions, Python/Go support
- [ ] **v0.3.0**: Vulnerability detection (RustSec, npm audit, OSV)
- [ ] **v1.0.0**: Publication to Zed Extensions marketplace

## Contributing

Contributions are welcome! Please see [TASKS.md](TASKS.md) for the current task list and priorities.

## License

MIT - See [LICENSE](LICENSE)

## Acknowledgments

- Inspired by [Dependi for VS Code](https://github.com/filllabs/dependi)
- Built with [tower-lsp](https://github.com/ebkalderon/tower-lsp)
- Thanks to the Zed team for the excellent extension API
