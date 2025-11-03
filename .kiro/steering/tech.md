# Technology Stack

## Language & Runtime

- **Rust**: Edition 2024, minimum version 1.75+
- **Async Runtime**: Tokio with full features for concurrent operations
- **Actor Pattern**: Used for database operations (DuckDB)

## Build System

**Cargo Workspace** with three main crates:
- `nuwax-cli`: CLI interface and command handlers
- `client-core`: Core business logic library
- `cli-ui`: Tauri-based GUI application (in development)

## Key Dependencies

### Core Libraries
- `tokio`: Async runtime with full feature set
- `clap`: Command-line argument parsing with derive macros
- `anyhow`/`thiserror`: Error handling
- `serde`/`serde_json`: Serialization
- `tracing`/`tracing-subscriber`: Structured logging

### Docker & Container Management
- `bollard`: Docker API client (v0.19)
- `ducker`: Integrated Docker TUI
- `docker-compose-types`: Parse docker-compose files

### Database & Storage
- `duckdb`: Embedded database with bundled features
- `tokio-stream`: Message passing for actor pattern

### File Operations
- `zip`/`zip-extract`: Archive handling
- `tar`/`flate2`: Compression
- `walkdir`: Directory traversal
- `tempfile`: Temporary file management

### Networking
- `reqwest`: HTTP client with rustls-tls
- `mysql_async`: MySQL client for SQL diff operations

### CLI UI (Tauri App)
- **Frontend**: React 18 + TypeScript + Vite
- **Styling**: TailwindCSS
- **Backend**: Tauri v2 with Rust
- **Package Manager**: Yarn

## Common Commands

### Development
```bash
# Check all workspace crates
cargo check --workspace

# Run tests
cargo test --workspace

# Format code
cargo fmt --all

# Lint with clippy
cargo clippy --workspace -- -D warnings

# Build release
cargo build --release
```

### Running
```bash
# Development mode
cargo run -- --help

# Production binary
./target/release/nuwax-cli --help

# Run specific command
cargo run -- status
```

### Testing
```bash
# Run all tests
cargo test --workspace

# Run specific test
cargo test --test mysql_integration_test

# Run with output
cargo test -- --nocapture
```

### Performance
```bash
# Run benchmarks
cargo bench

# Generate performance reports
cargo bench -- --output-format html
```

### GUI Development
```bash
cd cli-ui

# Install dependencies
yarn install

# Run dev server
yarn dev

# Build for production
yarn build

# Run Tauri app
yarn tauri dev
```

## Architecture Patterns

- **Layered Architecture**: Clear separation between CLI, business logic, and data access
- **Dependency Injection**: Unified component lifecycle via `CliApp`
- **Strategy Pattern**: Multiple upgrade strategies (full, patch, legacy)
- **Actor Pattern**: Concurrent-safe database operations

## Version Management

Four-segment versioning: `major.minor.patch.build`
- Example: `0.0.13.2` where `.2` is the patch level
- Base version: `0.0.13`
- Patch level: `2`
