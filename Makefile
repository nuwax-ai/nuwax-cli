.PHONY: build release dev install clean test fmt clippy check help package-linux package-linux-x86 package-linux-arm64

# Default target
help:
	@echo "Nuwax CLI Makefile"
	@echo ""
	@echo "Usage:"
	@echo "  make build      - Build nuwax-cli (release)"
	@echo "  make release    - Build nuwax-cli (release)"
	@echo "  make dev        - Build nuwax-cli (debug)"
	@echo "  make install    - Install nuwax-cli to ~/.cargo/bin"
	@echo "  make clean      - Clean build artifacts"
	@echo "  make test       - Run all tests"
	@echo "  make fmt        - Format code"
	@echo "  make clippy     - Run clippy linter"
	@echo "  make check      - Check workspace"
	@echo "  make package-linux       - Build and package Linux (x86_64 + arm64) binaries"
	@echo "  make package-linux-x86    - Build and package Linux x86_64 binary only"
	@echo "  make package-linux-arm64 - Build and package Linux ARM64 binary only"
	@echo ""
	@echo "Linux Build Dependencies:"
	@echo "  sudo apt-get update && sudo apt-get install -y --no-install-recommends \\"
	@echo "    build-essential pkg-config libglib2.0-dev libssl-dev curl wget file \\"
	@echo "    gcc-aarch64-linux-gnu"

# Build release version
build release:
	cargo build --release -p nuwax-cli

# Build debug version
dev:
	cargo build -p nuwax-cli

# Install nuwax-cli to ~/.cargo/bin
install:
	cargo install --path nuwax-cli --force

# Clean build artifacts
clean:
	cargo clean

# Run tests
test:
	cargo test --workspace

# Format code
fmt:
	cargo fmt --all

# Run clippy
clippy:
	cargo clippy --workspace -- -D warnings

# Check workspace
check:
	cargo check --workspace

# Build and package Linux x86_64 binary
# On Linux x86_64: native build; On macOS/other: uses cross
package-linux-x86:
	@mkdir -p dist
	@if [ "$(shell uname -s)" = "Linux" ] && [ "$(shell uname -m)" = "x86_64" ]; then \
		echo "=== Native Linux x86_64 build ==="; \
		if command -v apt-get > /dev/null 2>&1; then \
			sudo -n apt-get update > /dev/null 2>&1 && sudo -n apt-get install -y --no-install-recommends build-essential pkg-config libglib2.0-dev libssl-dev || echo "⚠️ Cannot install deps (need sudo), continuing anyway..."; \
		fi; \
		cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli; \
		tar czf dist/nuwax-cli-linux-amd64.tar.gz -C target/x86_64-unknown-linux-gnu/release nuwax-cli; \
	else \
		echo "=== Cross-compile for Linux x86_64 (macOS/non-x86_64 Linux) ==="; \
		command -v cross > /dev/null 2>&1 || { echo "Installing cross..."; cargo install cross --git https://github.com/cross-rs/cross; }; \
		cross build --release --target x86_64-unknown-linux-gnu -p nuwax-cli; \
		tar czf dist/nuwax-cli-linux-amd64.tar.gz -C target/x86_64-unknown-linux-gnu/release nuwax-cli; \
	fi
	@echo "✅ Packaged dist/nuwax-cli-linux-amd64.tar.gz"

# Build and package Linux ARM64 binary
# On Linux x86_64: native cross-compile with gcc-aarch64-linux-gnu; On macOS: uses cross
package-linux-arm64:
	@mkdir -p dist
	@if [ "$(shell uname -s)" = "Linux" ] && [ "$(shell uname -m)" = "x86_64" ]; then \
		echo "=== Cross-compile ARM64 on Linux x86_64 ==="; \
		if command -v apt-get > /dev/null 2>&1; then \
			sudo -n apt-get update > /dev/null 2>&1 && sudo -n apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu || echo "⚠️ Cannot install gcc-aarch64-linux-gnu (need sudo), continuing anyway..."; \
		fi; \
		cargo build --release --target aarch64-unknown-linux-gnu -p nuwax-cli; \
		tar czf dist/nuwax-cli-linux-arm64.tar.gz -C target/aarch64-unknown-linux-gnu/release nuwax-cli; \
	else \
		echo "=== Cross-compile ARM64 (macOS/non-x86_64 Linux) ==="; \
		command -v cross > /dev/null 2>&1 || { echo "Installing cross..."; cargo install cross --git https://github.com/cross-rs/cross; }; \
		cross build --release --target aarch64-unknown-linux-gnu -p nuwax-cli; \
		tar czf dist/nuwax-cli-linux-arm64.tar.gz -C target/aarch64-unknown-linux-gnu/release nuwax-cli; \
	fi
	@echo "✅ Packaged dist/nuwax-cli-linux-arm64.tar.gz"

# Build and package Linux (x86_64 + arm64)
package-linux:
	@mkdir -p dist
	@echo "Building x86_64..."
	@$(MAKE) package-linux-x86 &
	@echo "Building arm64..."
	@$(MAKE) package-linux-arm64 &
	@wait
	@echo "✅ All Linux packages built:"
	@ls -la dist/nuwax-cli-linux-*.tar.gz
