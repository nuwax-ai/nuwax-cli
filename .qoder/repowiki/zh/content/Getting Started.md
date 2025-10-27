# Getting Started

<cite>
**Referenced Files in This Document**   
- [README.md](file://README.md)
- [spec/cli-ui.md](file://spec/cli-ui.md)
- [cli-ui/src-tauri/Cargo.toml](file://cli-ui/src-tauri/Cargo.toml)
- [cli-ui/package.json](file://cli-ui/package.json)
- [nuwax-cli/Cargo.toml](file://nuwax-cli/Cargo.toml)
- [client-core/Cargo.toml](file://client-core/Cargo.toml)
- [cli-ui/tauri.conf.json](file://cli-ui/src-tauri/tauri.conf.json)
- [cli-ui/src-tauri/src/commands/cli.rs](file://cli-ui/src-tauri/src/commands/cli.rs)
- [cli-ui/src-tauri/src/main.rs](file://cli-ui/src-tauri/src/main.rs)
- [nuwax-cli/src/main.rs](file://nuwax-cli/src/main.rs)
- [nuwax-cli/src/commands/init.rs](file://nuwax-cli/src/commands/init.rs)
- [nuwax-cli/src/commands/status.rs](file://nuwax-cli/src/commands/status.rs)
- [nuwax-cli/src/commands/update.rs](file://nuwax-cli/src/commands/update.rs)
</cite>

## Table of Contents
1. [System Requirements](#system-requirements)
2. [Installation Instructions](#installation-instructions)
3. [Quick Start Tutorial](#quick-start-tutorial)
4. [Project Initialization](#project-initialization)
5. [Common Setup Issues](#common-setup-issues)
6. [Troubleshooting Guide](#troubleshooting-guide)
7. [Prerequisite Knowledge Resources](#prerequisite-knowledge-resources)

## System Requirements

### Supported Operating Systems
The duck_client project supports the following operating systems:
- **macOS**: Version 10.15 (Catalina) and later
- **Linux**: Most major distributions (Ubuntu, Debian, Fedora, CentOS, etc.)
- **Windows**: Windows 10 and Windows 11

### Required Dependencies
To use and contribute to duck_client, you need to install the following dependencies:

**Core Dependencies:**
- **Rust Toolchain**: Version 1.75 or later
  - Install via: `rustup update`
  - Verify installation: `rustc --version`
- **Node.js**: Version 22 or later (required for GUI development)
  - Install via package manager or from [nodejs.org](https://nodejs.org)
  - Verify installation: `node --version`
- **Docker**: Version 28.10 or later with Docker Compose
  - Install from [docker.com](https://www.docker.com)
  - Verify installation: `docker --version` and `docker-compose --version`

**Build Tools:**
- **Cargo**: Rust's package manager and build system (installed with Rust)
- **npm**: Node package manager (installed with Node.js)
- **Vite**: Frontend build tool (installed automatically with npm)

### Hardware Architecture Support
duck_client provides native support for both major CPU architectures:

**x86_64 (AMD64):**
- Intel and AMD processors
- Common in most desktop and laptop computers
- Used in Windows, macOS, and Linux systems

**aarch64 (ARM64):**
- Apple Silicon M1/M2/M3 processors
- ARM-based servers and devices
- Increasingly common in modern computing devices

**Architecture Detection:**
The system automatically detects your architecture during installation:
- Uses `TARGET` environment variable in build scripts
- Downloads appropriate binaries for your platform
- Supports universal builds for macOS (both Intel and Apple Silicon)

**Section sources**
- [README.md](file://README.md#L1-L92)
- [spec/cli-ui.md](file://spec/cli-ui.md#L200-L400)
- [cli-ui/src-tauri/Cargo.toml](file://cli-ui/src-tauri/Cargo.toml#L1-L50)

## Installation Instructions

### GUI Application (cli-ui) Installation

The GUI application is built using Tauri with React frontend and Rust backend.

**Step 1: Navigate to the cli-ui directory**
```bash
cd cli-ui
```

**Step 2: Install Node.js dependencies**
```bash
npm install
```

**Step 3: Start the development server**
```bash
npm run tauri dev
```
This command starts both the Vite development server and the Tauri application in development mode.

**Step 4: Build the production version**
```bash
npm run tauri build
```
This creates a production-ready application bundle for your platform.

**Alternative build commands:**
```bash
# Build only the frontend
npm run build

# Run type checking
npm run tsc

# Preview the built application
npm run preview
```

### CLI Tool (nuwax-cli) Installation

The command-line interface is built with Rust and Cargo.

**Step 1: Build the CLI tool**
```bash
# From the project root directory
cargo build -p nuwax-cli

# For release build with optimizations
cargo build --release -p nuwax-cli
```

**Step 2: Install the CLI tool globally**
```bash
# Install in your Cargo bin directory
cargo install --path nuwax-cli

# Verify installation
nuwax-cli --version
```

**Step 3: Alternative installation methods**
```bash
# Build and run without installing
cargo run -p nuwax-cli -- --help

# Run specific commands
cargo run -p nuwax-cli -- init
```

### Development Environment Setup

For contributors, set up the complete development environment:

**Step 1: Update Rust toolchain**
```bash
rustup update
rustup component add clippy rustfmt
```

**Step 2: Install workspace dependencies**
```bash
# From the project root
cargo check
cargo test
```

**Step 3: Set up IDE/Editor**
- Install Rust Analyzer for Rust code
- Install TypeScript support for frontend code
- Configure Prettier for code formatting

**Step 4: Verify the complete setup**
```bash
# Check all components
rustc --version
node --version  
docker --version
cargo --version
npm --version
```

**Section sources**
- [README.md](file://README.md#L45-L92)
- [spec/cli-ui.md](file://spec/cli-ui.md#L2200-L2400)
- [cli-ui/package.json](file://cli-ui/package.json#L1-L30)
- [nuwax-cli/Cargo.toml](file://nuwax-cli/Cargo.toml#L1-L20)

## Quick Start Tutorial

This tutorial demonstrates how to initialize, check status, and perform your first upgrade using both the GUI and CLI interfaces.

### Using the GUI Application

**Step 1: Launch the GUI**
```bash
cd cli-ui
npm run tauri dev
```

**Step 2: Set up your working directory**
1. When the application launches, you'll see a welcome modal
2. Click "Browse..." and select an empty directory for your project
3. Click "Confirm and Start" to initialize

**Step 3: Initialize the project**
1. In the operation panel, click the "Initialize" button (🚀 icon)
2. The terminal window will show the initialization process
3. Wait for the confirmation message: "✅ Initialization complete"

**Step 4: Check status**
1. Click the "Status" button (🔍 icon)
2. Observe the output in the terminal window
3. You should see information about your Docker service status

**Step 5: Perform your first upgrade**
1. Click the "Upgrade Service" button (⬆️ icon)
2. The application will check for updates and download if available
3. Watch the progress in the terminal window
4. Wait for the completion message

### Using the CLI Tool

**Step 1: Open a terminal**
Navigate to your project directory where you want to initialize duck_client.

**Step 2: Initialize the project**
```bash
# Run the init command
nuwax-cli init

# Expected output:
# [INFO] Initializing duck_client project
# [INFO] Creating configuration files
# [INFO] Setup complete - ready to use
```

**Step 3: Check the current status**
```bash
# Check Docker service status
nuwax-cli status

# Expected output:
# [STATUS] Docker service: Running
# [STATUS] Current version: v1.0.0
# [STATUS] Working directory: /path/to/your/project
```

**Step 4: Perform your first upgrade**
```bash
# Check for available updates
nuwax-cli check-update

# If updates are available, perform the upgrade
nuwax-cli upgrade --full

# Expected output:
# [INFO] Checking for updates...
# [INFO] New version available: v1.1.0
# [INFO] Downloading update package...
# [INFO] Applying upgrade...
# [SUCCESS] Upgrade completed successfully
# [INFO] New version: v1.1.0
```

**Step 5: Verify the upgrade**
```bash
# Check status again to verify the upgrade
nuwax-cli status

# The version should now show the updated version
```

### Command Reference

**Essential CLI Commands:**
```bash
# Project initialization
nuwax-cli init

# Check current status
nuwax-cli status

# Check for updates
nuwax-cli check-update

# Perform full upgrade
nuwax-cli upgrade --full

# Show help
nuwax-cli --help

# Show version
nuwax-cli --version
```

**GUI Operation Panel Buttons:**
| Button | Command | Description |
|--------|---------|-------------|
| 🚀 Initialize | `init` | Set up project configuration |
| 🔍 Check Update | `check-update` | Check for available updates |
| ⬆️ Upgrade | `upgrade --full` | Download and apply updates |
| ▶️ Start Service | `docker-service start` | Start Docker service |
| ⏹️ Stop Service | `docker-service stop` | Stop Docker service |
| 💾 Backup | `backup` | Create a backup of current state |

**Section sources**
- [README.md](file://README.md#L45-L92)
- [spec/cli-ui.md](file://spec/cli-ui.md#L800-L1200)
- [nuwax-cli/src/main.rs](file://nuwax-cli/src/main.rs#L1-L50)
- [nuwax-cli/src/commands/init.rs](file://nuwax-cli/src/commands/init.rs#L1-L30)
- [nuwax-cli/src/commands/status.rs](file://nuwax-cli/src/commands/status.rs#L1-L30)
- [nuwax-cli/src/commands/update.rs](file://nuwax-cli/src/commands/update.rs#L1-L30)

## Project Initialization

### Initialization via CLI Command

The `init` command sets up your project with default configuration.

**Step 1: Run the init command**
```bash
nuwax-cli init
```

**Step 2: Follow the interactive prompts**
```text
Welcome to duck_client initialization!
Please answer the following questions:

Working directory [/current/path]:
Database type (mysql/postgresql) [mysql]: 
Docker service name [duck-service]: 
Enable automatic backups? (y/n) [y]: 
```

**Step 3: Understand the created files**
The init command creates the following structure:
```
your-project/
├── .duck_client/
│   ├── config.json
│   ├── logs/
│   └── backups/
├── docker-compose.yml
└── README.md
```

**Configuration options:**
- **config.json**: Main configuration file with service settings
- **docker-compose.yml**: Docker service definitions
- **logs/**: Directory for application logs
- **backups/**: Directory for backup files

### Initialization via GUI Setup Modal

The GUI provides a visual setup process for new users.

**Step 1: Launch the GUI application**
```bash
cd cli-ui
npm run tauri dev
```

**Step 2: Complete the welcome setup modal**
```
┌─────────────────────────────────────────┐
│              🦆 Duck CLI GUI            │
│                                         │
│   Welcome! Please select working dir:   │
│   ┌─────────────────┐ [Browse...]       │
│   │ /path/to/dir    │                   │
│   └─────────────────┘                   │
│                                         │
│   💡 Tips:                              │
│   • Use an empty directory              │
│   • Ensure read/write permissions       │
│   • Avoid system directories            │
│                                         │
│   [Skip]  [Confirm and Start]           │
└─────────────────────────────────────────┘
```

**Step 3: Configure basic settings**
After selecting the directory, you'll see additional configuration options:
- **Service Type**: Select Docker service template
- **Resource Allocation**: Set CPU and memory limits
- **Network Settings**: Configure ports and network mode
- **Persistence**: Enable/disable data persistence

**Step 4: Review and confirm**
The final screen shows a summary of your configuration:
```
Configuration Summary:
- Working Directory: /Users/username/duck-project
- Service Type: MySQL Database
- CPU Limit: 2 cores
- Memory Limit: 4GB
- Port Mapping: 3306:3306
- Data Persistence: Enabled

[Back] [Finish Setup]
```

### Configuration Management

Both CLI and GUI methods create the same underlying configuration.

**Main configuration file (.duck_client/config.json):**
```json
{
  "version": "1.0",
  "working_directory": "/path/to/project",
  "docker_service": {
    "name": "duck-service",
    "image": "duck-service:latest",
    "ports": [3306],
    "volumes": ["data:/var/lib/mysql"]
  },
  "backup": {
    "enabled": true,
    "schedule": "daily",
    "retention": 7
  },
  "update": {
    "auto_check": true,
    "notification": "desktop"
  }
}
```

**Configuration precedence:**
1. Command-line arguments (highest priority)
2. Configuration file settings
3. Environment variables
4. Default values (lowest priority)

**Section sources**
- [spec/cli-ui.md](file://spec/cli-ui.md#L800-L1000)
- [nuwax-cli/src/commands/init.rs](file://nuwax-cli/src/commands/init.rs#L1-L50)
- [client-core/src/config.rs](file://client-core/src/config.rs#L1-L30)

## Common Setup Issues

### Docker Permissions Issues

**Problem:** Docker commands fail with permission errors.

**Symptoms:**
- "Cannot connect to the Docker daemon" error
- Permission denied when accessing Docker socket
- Commands timeout without response

**Solutions:**

**For Linux:**
```bash
# Add your user to the docker group
sudo usermod -aG docker $USER

# Log out and log back in, or run:
newgrp docker

# Verify access
docker ps
```

**For macOS and Windows:**
- Ensure Docker Desktop is running
- Check that Docker is set to start automatically
- Restart Docker Desktop if commands are not responding

**GUI-specific solution:**
If using the GUI application, ensure the Tauri app has permission to access Docker:
1. On macOS: Go to System Settings → Privacy & Security → Full Disk Access
2. Add the duck_client GUI application
3. Restart the application

### Architecture Detection Problems

**Problem:** The system downloads the wrong binary for your architecture.

**Symptoms:**
- "Exec format error" when running commands
- Binary not executable
- Application crashes on startup

**Solutions:**

**Verify your architecture:**
```bash
# Check system architecture
uname -m

# Expected outputs:
# x86_64 for Intel/AMD processors
# aarch64 for Apple Silicon ARM processors
```

**Manually specify architecture for build:**
```bash
# For x86_64
cargo build --target x86_64-unknown-linux-gnu -p nuwax-cli

# For aarch64
cargo build --target aarch64-unknown-linux-gnu -p nuwax-cli
```

**Clean and rebuild:**
```bash
# Clean the build directory
cargo clean

# Rebuild for your specific target
cargo build --release

# Or specify target explicitly
cargo build --target $(rustc --print target-list | grep $(uname -m) | head -1) -p nuwax-cli
```

### Network Configuration Issues

**Problem:** The application cannot connect to required services or repositories.

**Symptoms:**
- Timeout when downloading updates
- Unable to reach GitHub repositories
- Slow download speeds
- Certificate errors

**Solutions:**

**Check internet connection:**
```bash
# Test basic connectivity
ping github.com

# Test HTTPS access
curl -I https://github.com
```

**Configure proxy settings (if behind corporate firewall):**
```bash
# Set environment variables
export HTTP_PROXY=http://proxy.company.com:8080
export HTTPS_PROXY=http://proxy.company.com:8080
export NO_PROXY=localhost,127.0.0.1

# For Cargo specifically
# Create or edit ~/.cargo/config
[http]
proxy = "http://proxy.company.com:8080"
[https]
proxy = "http://proxy.company.com:8080"
```

**Update SSL certificates:**
```bash
# On Ubuntu/Debian
sudo apt update && sudo apt install ca-certificates

# On macOS with Homebrew
brew install ca-certificates
```

**Section sources**
- [README.md](file://README.md#L1-L92)
- [spec/cli-ui.md](file://spec/cli-ui.md#L200-L400)
- [nuwax-cli/src/docker_service/mod.rs](file://nuwax-cli/src/docker_service/mod.rs#L1-L30)

## Troubleshooting Guide

### Build Failures

**Problem:** Compilation fails during build process.

**Common error messages and solutions:**

**Missing Rust components:**
```text
error: toolchain 'stable-x86_64-unknown-linux-gnu' is not installed
```
**Solution:**
```bash
rustup update
rustup toolchain install stable
```

**Missing Node.js modules:**
```text
Error: Cannot find module 'react'
```
**Solution:**
```bash
cd cli-ui
npm install
# or
npm ci  # Clean install
```

**Cargo lock file conflicts:**
```text
error: multiple packages with name `serde` in this workspace
```
**Solution:**
```bash
cargo clean
cargo update
cargo build
```

**Platform-specific build issues:**
```text
linking with `cc` failed: exit status: 1
```
**Solution:**
```bash
# Install build essentials
# Ubuntu/Debian:
sudo apt install build-essential

# macOS:
xcode-select --install

# Windows:
# Install Visual Studio Build Tools
```

### Runtime Errors

**Problem:** Application starts but encounters errors during execution.

**Common issues and solutions:**

**Configuration file errors:**
```text
[ERROR] Failed to parse config.json: invalid syntax
```
**Solution:**
1. Check the configuration file for JSON syntax errors
2. Use a JSON validator to verify the file
3. Restore from backup if available

**Docker service not running:**
```text
[ERROR] Cannot connect to Docker daemon
```
**Solution:**
```bash
# Start Docker service
# Linux:
sudo systemctl start docker

# macOS/Windows:
# Start Docker Desktop application
```

**Permission denied errors:**
```text
[ERROR] Permission denied when writing to /path/to/file
```
**Solution:**
```bash
# Check file permissions
ls -la /path/to/file

# Fix permissions
chmod 644 /path/to/file
# or for directories
chmod 755 /path/to/directory

# Change ownership if needed
sudo chown $USER:$USER /path/to/file
```

### GUI-Specific Issues

**Problem:** Tauri GUI application fails to start or display properly.

**Solutions:**

**Clear Tauri cache:**
```bash
# Remove Tauri build artifacts
rm -rf cli-ui/src-tauri/target
rm -rf cli-ui/dist

# Rebuild the application
npm run tauri build
```

**Reset application data:**
```bash
# Remove application data
# macOS:
rm -rf ~/Library/Application\ Support/com.soddy.cli-ui

# Linux:
rm -rf ~/.local/share/com.soddy.cli-ui

# Windows:
# Remove from AppData/Roaming/com.soddy.cli-ui
```

**Debug GUI issues:**
```bash
# Run with debug logging
npm run tauri dev -- --debug

# Check browser console for frontend errors
# Press F12 in the application window
```

### Performance Issues

**Problem:** Slow performance during operations.

**Optimization tips:**

**For slow downloads:**
```bash
# Use a mirror for crates.io
# Add to ~/.cargo/config
[source.crates-io]
replace-with = 'tuna'
[source.tuna]
registry = "https://mirrors.tuna.tsinghua.edu.cn/git/crates.io-index.git"
```

**For high memory usage:**
```bash
# Limit Docker memory
# In docker-compose.yml
services:
  duck-service:
    mem_limit: 2g

# Or via CLI
nuwax-cli docker-service start --memory 2g
```

**For slow initialization:**
```bash
# Pre-download required images
docker pull duck-service:latest

# Use SSD storage for better I/O performance
```

**Section sources**
- [README.md](file://README.md#L45-L92)
- [spec/cli-ui.md](file://spec/cli-ui.md#L2000-L2400)
- [nuwax-cli/src/lib.rs](file://nuwax-cli/src/lib.rs#L1-L30)
- [client-core/src/error.rs](file://client-core/src/error.rs#L1-L20)

## Prerequisite Knowledge Resources

To effectively use and contribute to duck_client, familiarity with the following technologies is recommended:

### Docker Resources
- **Official Docker Documentation**: https://docs.docker.com
- **Docker Getting Started**: https://docs.docker.com/get-started/
- **Docker Compose Reference**: https://docs.docker.com/compose/compose-file/
- **Docker CLI Reference**: https://docs.docker.com/engine/reference/commandline/cli/

### Git Resources
- **Pro Git Book (Free)**: https://git-scm.com/book/en/v2
- **Git Documentation**: https://git-scm.com/docs
- **GitHub Skills (Interactive Learning)**: https://skills.github.com
- **Git Best Practices**: https://github.com/git-tips/tips

### Semantic Versioning
- **SemVer Specification**: https://semver.org
- **Semantic Versioning Guide**: https://docs.npmjs.com/about-semantic-versioning
- **Version Ranges**: https://docs.npmjs.com/cli/v8/using-npm/semver

### Rust Learning Resources
- **The Rust Programming Language (Book)**: https://doc.rust-lang.org/book/
- **Rust by Example**: https://doc.rust-lang.org/rust-by-example/
- **Rust Standard Library Documentation**: https://doc.rust-lang.org/std/
- **Cargo Documentation**: https://doc.rust-lang.org/cargo/

### Node.js and Frontend Development
- **Node.js Documentation**: https://nodejs.org/api/
- **React Documentation**: https://react.dev
- **Vite Documentation**: https://vitejs.dev
- **TypeScript Handbook**: https://www.typescriptlang.org/docs/

### Tauri Framework
- **Tauri Documentation**: https://tauri.app
- **Tauri API Reference**: https://docs.rs/tauri/latest/tauri/
- **Tauri Plugins**: https://plugins.tauri.app