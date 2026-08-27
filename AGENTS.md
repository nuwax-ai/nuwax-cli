# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Project Overview

Nuwax CLI is a Rust-based Docker service management and upgrade tool with a modular workspace architecture:

- **nuwax-cli**: Main CLI binary entry point (commands, docker service management, TUI)
- **client-core**: Shared core business logic library (API, database, upgrade, SQL diff)

Edition: 2024

## Development Commands

### Building
```bash
# Build CLI (release)
cargo build --release -p nuwax-cli

# Build workspace
cargo build --workspace
```

### Testing
```bash
# Run all workspace tests (preferred, requires cargo-nextest)
cargo nextest run

# Run tests for specific package
cargo nextest run -p client-core

# Fallback if nextest is not installed
cargo test --workspace

# Run benchmarks
cargo bench
```

### Code Quality
```bash
# Format all code
cargo fmt --all

# Lint workspace
cargo clippy --workspace

# Check workspace integrity
cargo check --workspace
```

### Dependency Management
```bash
# Check for dependency upgrades (requires cargo-edit)
cargo upgrade --dry-run

# Upgrade dependencies in Cargo.toml
cargo upgrade

# Update Cargo.lock to latest compatible versions
cargo update
```

### Running Applications
```bash
# Run CLI in development
cargo run -- --help

# Run CLI with debug logging
RUST_LOG=debug cargo run -- status
```

## Architecture

### Core Components
- **UpgradeStrategy**: Strategy pattern for service upgrades (full/incremental/patch)
- **PatchExecutor**: Applies incremental patches with file operations and rollback support
- **BackupManager** (`backup.rs`): Data backup and restoration with compression
- **DockerServiceManager** (`docker_service/manager.rs`): Docker container lifecycle management via Bollard API
- **DatabaseManager** (`database_manager.rs`): Embedded DuckDB for state persistence and analytics
- **ApiClient** (`api.rs` + `authenticated_client.rs`): Remote API communication for version checks and downloads
- **SqlDiff** (`sql_diff/`): SQL schema diff engine for MySQL DDL comparison and migration generation
- **Ducker** (`commands/ducker.rs`): Integrated Docker TUI (terminal UI) for container management
- **Architecture** (`architecture.rs`): Multi-architecture support (x86_64/aarch64)

### Key Patterns
- **Workspace Architecture**: Shared dependencies in root Cargo.toml, package-specific overrides
- **Async/Await**: Tokio runtime throughout, concurrent operations where beneficial
- **Error Handling**: anyhow + thiserror for comprehensive error management
- **Configuration**: Smart config discovery with fallback search paths
- **Cross-platform**: Platform-specific logic isolated in constants and container/environment modules
- **Internationalization**: rust-i18n with locale files in `locales/` directory
- **TUI**: Ratatui-based terminal UI for interactive operations

### Data Flow
1. CLI commands → CliApp → Business Logic (client-core) → External Systems (Docker/API)
2. Configuration loaded hierarchically: CLI args → config.toml → defaults
3. State persisted in DuckDB with backup/restore capabilities

## CLI Commands

| Command | Description |
|---------|-------------|
| `status` | Show service status and version information |
| `init` | Initialize client, create config file and database |
| `check-update` | Check for client updates |
| `api-info` | Show current API configuration |
| `upgrade` | Download and apply service upgrades (subcommands: check, download) |
| `list-backups` | List all backups |
| `rollback` | Restore from backup with optional data rollback |
| `docker-service` | Docker service management (start, stop, restart, logs, etc.) |
| `ducker` | Integrated Docker TUI for container management |
| `auto-backup` | Auto backup scheduling and management |
| `auto-upgrade-deploy` | Automated upgrade and deployment |
| `cache` | Cache management |
| `diff-sql` | Compare two SQL files and generate diff migration SQL |

## Important Implementation Details

### Constants
All project constants are centralized in `client-core/src/constants.rs` including:
- Docker paths and environment variables
- API endpoints and timeouts
- File format definitions
- Version information

### Docker Integration
- Uses Bollard crate for Docker API communication (locked to v0.20.1 for ducker compatibility)
- Supports both Docker Compose v2 and direct container management
- Cross-platform Docker socket paths (Unix vs Windows)
- Health checking with configurable timeouts
- Multi-architecture image support (x86_64/aarch64)

### Database Operations
- DuckDB for embedded analytics and state storage (bundled build)
- MySQL support for SQL diff execution and schema upgrades
- Dashmap for concurrent-safe in-memory data structures (instead of Arc<RwLock<HashMap>>)
- SQL diff engine supports FULLTEXT/SPATIAL index parsing via custom sqlparser fork (nuwax-sqlparser)

### Patch System
- `PatchExecutor` handles incremental upgrades with atomic file operations
- Supports backup before apply, rollback on failure
- SHA256 hash verification for patch integrity
- tar.gz and zip archive extraction

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
- Use DockerServiceManager abstraction, never direct Bollard calls
- Follow timeout constants from `constants.rs`
- Implement proper error handling for Docker daemon connectivity

### Database Schema Changes
- Update `client-core/src/database/` modules
- Ensure backward compatibility with existing DuckDB files
- Add migration logic if needed

### SQL Diff Development
- SQL parsing uses `nuwax-sqlparser` (custom fork with FULLTEXT/SPATIAL INDEX support)
- Test fixtures in `client-core/tests/fixtures/`
- Diff engine supports column, index, constraint, and comment changes

## Performance Considerations

- Use `dashmap` instead of `Arc<RwLock<HashMap>>` for concurrent-safe data structures
- Leverage Tokio's concurrency for async operations
- Implement proper resource cleanup with Drop traits
- Use connection pooling for database operations
- DuckDB bundled build for zero external dependencies
