# Nuwax CLI - Intelligent Docker Service Management Tool

<div align="center">

![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)
![Docker](https://img.shields.io/badge/docker-20.10+-blue.svg)
![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-green.svg)

A professional **Docker service management and upgrade tool** providing complete containerized service lifecycle management.

</div>


[中文文档](README.zh-CN.md)

## 🎯 Project Overview

Nuwax CLI is a modern Docker service management tool developed in Rust, specifically designed to simplify the deployment, upgrade, backup, and maintenance of containerized applications. Through intelligent upgrade strategies and robust security mechanisms, it provides reliable operational support for enterprise-level applications.

### ✨ Core Features

- **🐋 Intelligent Docker Management**: Complete Docker container lifecycle management with start, stop, restart, and health check capabilities
- **🔄 Multi-Strategy Upgrades**: Support for both full and incremental upgrades with automatic optimal strategy selection
- **💾 Complete Backup System**: Automatic backup before upgrades with full rollback support for data and application configurations
- **🏗️ Cross-Platform Architecture**: Native support for x86_64 and aarch64 architectures with automatic system type detection
- **📊 Real-time Monitoring**: Service status monitoring, health checks, and performance metrics collection
- **🛡️ Secure & Reliable**: Transactional upgrade operations with automatic rollback on failure to ensure service stability
- **⚡ High Performance**: Built on Rust async runtime providing exceptional concurrent performance
- **🎨 Modern CLI**: Intuitive command-line interface with rich progress displays and status indicators

## 📁 Project Architecture

```
nuwax-cli/
├── 📦 nuwax-cli/          # CLI Main Program
│   ├── src/
│   │   ├── main.rs        # Program Entry Point
│   │   ├── cli.rs         # Command Line Definitions
│   │   ├── app.rs         # Application Main Logic
│   │   ├── commands/      # Command Processors
│   │   └── docker_service/ # Docker Service Management
│   └── Cargo.toml
├── 🔧 client-core/        # Core Business Library
│   ├── src/
│   │   ├── upgrade.rs     # Upgrade Management
│   │   ├── backup.rs      # Backup System
│   │   ├── database.rs    # Database Management
│   │   ├── api.rs         # API Client
│   │   ├── container/     # Docker Operations
│   │   └── sql_diff/      # SQL Diff Comparison
│   └── Cargo.toml
├── 🖥️ cli-ui/            # Tauri GUI Application (In Development)
│   ├── src-tauri/        # Tauri Backend
│   └── src/              # Frontend Interface
├── 📚 docs/              # Technical Documentation
├── 📋 spec/              # Design Specifications
├── 🗄️ data/              # Data Directory
└── 📄 README.md
```

## 🚀 Quick Start

### Requirements

- **Rust**: 1.75+
- **Docker**: 20.10+ and Docker Compose v2+
- **Operating System**: Windows 10+, macOS 10.15+, Linux (mainstream distributions)
- **Memory**: Minimum 512MB available memory

### Installation

#### Build from Source

```bash
# Clone repository
git clone https://github.com/soddygo/nuwax-cli.git
cd nuwax-cli

# Build project
cargo build --release

# Install to system
cargo install --path .
```

#### Direct Run

```bash
# Development mode
cargo run -- --help

# Production mode
./target/release/nuwax-cli --help
```

### Basic Usage

```bash
# 1. Initialize working environment
nuwax-cli init

# 2. Check service status
nuwax-cli status

# 3. Download and deploy services
nuwax-cli upgrade

# 4. Start Docker services
nuwax-cli docker-service start

# 5. Create backup
nuwax-cli backup

# 6. Check available updates
nuwax-cli check-update check
```

## 📖 Detailed Features

### Docker Service Management

```bash
# Service Control
nuwax-cli docker-service start        # Start services
nuwax-cli docker-service stop         # Stop services
nuwax-cli docker-service restart      # Restart services
nuwax-cli docker-service status       # Check status

# Image Management
nuwax-cli docker-service load-images  # Load images
nuwax-cli docker-service arch-info    # Architecture info

# Utilities
nuwax-cli ducker                      # Launch Docker TUI
```

### Upgrade and Backup

```bash
# Upgrade Management
nuwax-cli upgrade                     # Execute upgrade
nuwax-cli upgrade --check            # Check updates
nuwax-cli upgrade --force           # Force reinstall

# Backup and Recovery
nuwax-cli backup                     # Create backup
nuwax-cli list-backups              # List backups
nuwax-cli rollback                  # Rollback recovery
nuwax-cli rollback --force         # Force rollback
```

### Automated Operations

```bash
# Auto Backup
nuwax-cli auto-backup run           # Immediate backup
nuwax-cli auto-backup status        # Backup status

# Auto Upgrade Deployment
nuwax-cli auto-upgrade-deploy run   # Auto upgrade deployment
nuwax-cli auto-upgrade-deploy status # View configuration
```

### Utility Commands

```bash
# SQL Diff Comparison
nuwax-cli diff-sql old.sql new.sql --old-version 1.0 --new-version 2.0

# Cache Management
nuwax-cli cache clear               # Clear cache
nuwax-cli cache status             # Cache status
```

## 🛠️ Development Guide

### Development Environment Setup

```bash
# 1. Install Rust toolchain
rustup update stable
rustup component add rustfmt clippy

# 2. Verify dependencies
cargo check --workspace

# 3. Run tests
cargo test --workspace

# 4. Code formatting
cargo fmt --all

# 5. Static analysis
cargo clippy --workspace -- -D warnings
```

### Performance Testing

```bash
# Run performance benchmarks
cargo bench

# Generate performance reports
cargo bench -- --output-format html
```

### Project Dependency Management

The project uses Cargo workspace to manage multiple sub-modules:

- **nuwax-cli**: CLI interface layer, depends on client-core
- **client-core**: Core business logic, independently testable
- **cli-ui**: Tauri GUI application, independent frontend project

All dependency versions are uniformly managed in the root `Cargo.toml` to ensure consistency.

## 🔧 Configuration

### Configuration File Structure

The project uses `config.toml` configuration files with intelligent configuration discovery:

```toml
[versions]
docker_service = "1.0.0"
patch_version = "1.0.1"
full_version_with_patches = "1.0.1+1"

[docker]
compose_file = "docker/docker-compose.yml"
env_file = "docker/.env"

[backup]
storage_dir = "./backups"
max_backups = 10

[cache]
download_dir = "./cache"
max_cache_size = "1GB"

[updates]
auto_check = true
auto_backup = true
```

### Intelligent Configuration Discovery

Configuration file search order:
1. Command line specified path (`--config`)
2. Current directory `./config.toml`
3. Recursive search to parent directories
4. User home directory `~/.nuwax/config.toml`

## 🏗️ System Architecture

### Core Components

- **CLI Interface Layer**: Command parsing, user interaction, progress display
- **Business Logic Layer**: Upgrade strategies, backup management, Docker operations
- **Data Access Layer**: DuckDB storage, configuration management, state persistence
- **API Client**: Version checking, file downloading, service communication

### Design Patterns

- **Layered Architecture**: Clear separation of responsibilities and dependency management
- **Dependency Injection**: Unified component lifecycle management through `CliApp`
- **Strategy Pattern**: Flexible switching between multiple upgrade strategies
- **Actor Pattern**: Concurrent safe processing of database operations

## 🤝 Contributing

We welcome community contributions! Please follow these steps:

1. **Fork** the project to your GitHub account
2. **Create** a feature branch (`git checkout -b feature/amazing-feature`)
3. **Commit** your changes (`git commit -m 'Add some amazing feature'`)
4. **Push** to the branch (`git push origin feature/amazing-feature`)
5. **Create** a Pull Request

### Code Standards

- Use `cargo fmt` for code formatting
- Use `cargo clippy` for static checking
- Add unit tests for new features
- Update relevant documentation

## 📄 License

This project is dual-licensed:

- [MIT License](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)

You may choose either license to use this project.

## 🔗 Related Links

- **Project Homepage**: https://docx.xspaceagi.com/
- **GitHub Repository**: https://github.com/soddygo/nuwax-cli
- **Issue Reporting**: https://github.com/soddygo/nuwax-cli/issues
- **Changelog**: [CHANGELOG.md](CHANGELOG.md)

## 💬 Support

If you encounter issues or have improvement suggestions:

1. Check [documentation](docs/) for detailed information
2. Search [known issues](https://github.com/soddygo/nuwax-cli/issues)
3. Create a new [Issue](https://github.com/soddygo/nuwax-cli/issues/new)
4. Join [discussions](https://github.com/soddygo/nuwax-cli/discussions)

---

<div align="center">

**[⬆ Back to Top](#nuwax-cli---intelligent-docker-service-management-tool)**

Made with ❤️ by the Nuwax Team

</div>
