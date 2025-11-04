# Project Structure

## Workspace Layout

```
nuwax-cli/                    # Root workspace
├── nuwax-cli/                # CLI main program
├── client-core/              # Core business library
├── cli-ui/                   # Tauri GUI application
├── docs/                     # Technical documentation
├── spec/                     # Design specifications
├── data/                     # Runtime data directory
└── tmp/                      # Temporary files
```

## nuwax-cli/ - CLI Application

```
nuwax-cli/
├── src/
│   ├── main.rs              # Entry point, command routing
│   ├── cli.rs               # Clap command definitions
│   ├── app.rs               # CliApp - main application logic
│   ├── commands/            # Command implementations
│   │   ├── mod.rs
│   │   ├── auto_backup.rs
│   │   ├── auto_upgrade_deploy.rs
│   │   ├── backup.rs
│   │   ├── cache.rs
│   │   ├── check_update.rs
│   │   ├── diff_sql.rs
│   │   ├── docker_service.rs
│   │   ├── ducker.rs
│   │   ├── status.rs
│   │   └── update.rs
│   ├── docker_service/      # Docker service management
│   │   ├── manager.rs       # Main service manager
│   │   ├── config.rs        # Configuration handling
│   │   ├── architecture.rs  # Architecture detection
│   │   ├── compose_parser.rs
│   │   ├── health_check.rs
│   │   ├── image_loader.rs
│   │   └── service_manager.rs
│   ├── init.rs              # Initialization logic
│   ├── project_info.rs      # Version and metadata
│   └── ui_support.rs        # UI helper functions
├── benches/                 # Performance benchmarks
└── tests/                   # Integration tests
```

## client-core/ - Core Library

```
client-core/
├── src/
│   ├── lib.rs               # Public API exports
│   ├── api.rs               # API client
│   ├── api_config.rs        # API configuration
│   ├── api_types.rs         # API data structures
│   ├── architecture.rs      # Architecture detection
│   ├── authenticated_client.rs
│   ├── backup.rs            # Backup system
│   ├── config.rs            # Configuration structures
│   ├── config_manager.rs    # Config file management
│   ├── constants.rs         # Global constants
│   ├── container/           # Docker operations
│   │   ├── mod.rs
│   │   ├── command.rs       # Docker commands
│   │   ├── config.rs        # Container config
│   │   ├── image.rs         # Image management
│   │   ├── modern_docker.rs # Modern Docker API
│   │   ├── service.rs       # Service operations
│   │   ├── types.rs         # Type definitions
│   │   └── volumes.rs       # Volume management
│   ├── database.rs          # Database operations
│   ├── database_manager.rs  # DB lifecycle
│   ├── db/                  # Actor-based DB access
│   │   ├── actor.rs         # Database actor
│   │   ├── manager.rs       # Actor manager
│   │   ├── messages.rs      # Message types
│   │   └── models.rs        # Data models
│   ├── downloader.rs        # File download
│   ├── error.rs             # Error types
│   ├── mysql_executor.rs    # MySQL operations
│   ├── patch_executor/      # Incremental upgrades
│   │   ├── mod.rs           # Main executor
│   │   ├── error.rs         # Patch errors
│   │   ├── file_operations.rs
│   │   └── patch_processor.rs
│   ├── sql_diff/            # SQL comparison
│   │   ├── mod.rs
│   │   ├── differ.rs        # Diff algorithm
│   │   ├── generator.rs     # SQL generation
│   │   ├── parser.rs        # SQL parsing
│   │   ├── types.rs         # Type definitions
│   │   └── tests.rs         # Unit tests
│   ├── upgrade.rs           # Upgrade orchestration
│   ├── upgrade_strategy.rs  # Strategy selection
│   └── version.rs           # Version management
├── fixtures/                # Test fixtures
├── migrations/              # Database migrations
├── templates/               # Config templates
└── tests/                   # Integration tests
```

## cli-ui/ - GUI Application

```
cli-ui/
├── src/                     # React frontend
│   ├── App.tsx              # Main app component
│   ├── components/          # React components
│   │   ├── BackupSelectionModal.tsx
│   │   ├── ErrorBoundary.tsx
│   │   ├── OperationPanel.tsx
│   │   ├── ParameterInputModal.tsx
│   │   ├── TerminalWindow.tsx
│   │   ├── WelcomeSetupModal.tsx
│   │   └── WorkingDirectoryBar.tsx
│   ├── config/              # Frontend config
│   ├── types/               # TypeScript types
│   └── utils/               # Utility functions
├── src-tauri/               # Tauri backend
│   ├── src/
│   │   ├── main.rs          # Tauri entry point
│   │   ├── lib.rs           # Library exports
│   │   └── commands/        # Tauri commands
│   │       ├── cli.rs       # CLI integration
│   │       ├── config.rs    # Config management
│   │       └── mod.rs
│   ├── capabilities/        # Tauri permissions
│   └── icons/               # App icons
├── package.json             # Node dependencies
├── vite.config.ts           # Vite configuration
└── tailwind.config.js       # TailwindCSS config
```

## Key Conventions

### Module Organization
- Each command has its own file in `commands/`
- Complex features get their own subdirectory (e.g., `docker_service/`, `patch_executor/`)
- Tests are co-located with implementation or in `tests/` directory

### File Naming
- Snake_case for Rust files: `upgrade_strategy.rs`
- PascalCase for React components: `OperationPanel.tsx`
- Kebab-case for config files: `docker-compose.yml`

### Configuration Files
- `config.toml`: Main application configuration
- `Cargo.toml`: Rust dependencies and workspace config
- `.env`: Environment variables for Docker services
- `docker-compose.yml`: Docker service definitions

### Data Directories
- `data/`: Runtime data and DuckDB database
- `cache/`: Downloaded packages and temporary files
- `backups/`: Service backups for rollback
- `tmp/`: Temporary working files
