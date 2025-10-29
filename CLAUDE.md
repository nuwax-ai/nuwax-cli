# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Nuwax CLI is a Rust-based Docker service management and upgrade tool with a modular workspace architecture:

- **nuwax-cli**: Main CLI binary entry point
- **client-core**: Shared core business logic library  
- **cli-ui**: Tauri desktop GUI application (React + TypeScript)

## Development Commands

### Building
```bash
# Build CLI (release)
cargo build --release -p nuwax-cli

# Build Tauri GUI (must be in cli-ui directory)
cd cli-ui && npm run tauri build

# Build workspace
cargo build --workspace
```

### Testing
```bash
# Run all workspace tests
cargo test --workspace

# Run tests for specific package
cargo test -p client-core

# Run benchmarks
cargo bench
```

### Code Quality
```bash
# Format all code
cargo fmt --all

# Lint workspace (warnings as errors)
cargo clippy --workspace -- -D warnings

# Check workspace integrity
cargo check --workspace
```

### Running Applications
```bash
# Run CLI in development
cargo run -- --help

# Run Tauri GUI (must be in cli-ui directory)
cd cli-ui && npm run tauri dev

# Run CLI with debug logging
RUST_LOG=debug cargo run -- status
```

## Architecture

### Core Components
- **UpgradeManager**: Handles service upgrades with strategy pattern (full/incremental)
- **BackupManager**: Manages data backups and restoration with compression
- **DockerManager**: Docker container lifecycle management via Bollard API
- **DatabaseManager**: State persistence using DuckDB
- **ApiClient**: Remote API communication for version checks and downloads

### Key Patterns
- **Workspace Architecture**: Shared dependencies in root Cargo.toml, package-specific overrides
- **Async/Await**: Tokio runtime throughout, concurrent operations where beneficial
- **Error Handling**: anyhow + thiserror for comprehensive error management
- **Configuration**: Smart config discovery with fallback search paths
- **Cross-platform**: Platform-specific logic isolated in constants.rs

### Data Flow
1. CLI commands → CliApp → Business Logic (client-core) → External Systems (Docker/API)
2. Configuration loaded hierarchically: CLI args → config.toml → defaults
3. State persisted in DuckDB with backup/restore capabilities

## Important Implementation Details

### Constants
All project constants are centralized in `client-core/src/constants.rs` including:
- Docker paths and environment variables
- API endpoints and timeouts
- File format definitions
- Version information

### Docker Integration
- Uses Bollard crate for Docker API communication
- Supports both Docker Compose v2 and direct container management
- Cross-platform Docker socket paths (Unix vs Windows)
- Health checking with configurable timeouts

### Database Operations
- DuckDB for embedded analytics and state storage
- MySQL support for SQL diff execution and schema upgrades
- Dashmap for concurrent-safe in-memory data structures (instead of Arc<RwLock<HashMap>>)

### Tauri GUI Development
- Must run from `cli-ui/` directory, not workspace root
- Uses Tauri updater and file-system plugins
- Development server: `npm run tauri dev`
- API documentation available at: `http://127.0.0.1:3000/api-docs/openapi.json`

## Configuration Management

Configuration files are searched in order:
1. Command line specified path (`--config`)
2. Current directory `./config.toml`  
3. Parent directories (recursive search)
4. User home directory `~/.nuwax/config.toml`

## Common Development Workflows

### Adding New CLI Commands
1. Define command in `nuwax-cli/src/cli.rs`
2. Add command handler in `nuwax-cli/src/commands/`
3. Implement business logic in `client-core/src/`
4. Add tests for both CLI and core logic

### Working with Docker Operations
- Use DockerManager abstraction, never direct Bollard calls
- Follow timeout constants from `constants.rs`
- Implement proper error handling for Docker daemon connectivity

### Database Schema Changes
- Update `client-core/src/database/` modules
- Ensure backward compatibility with existing DuckDB files
- Add migration logic if needed