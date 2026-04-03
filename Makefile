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

# Check Linux build dependencies (fail fast if missing)
check-linux-deps:
	@if [ "$(shell uname -s)" = "Linux" ] && [ "$(shell uname -m)" = "x86_64" ]; then \
		if ! command -v aarch64-linux-gnu-gcc > /dev/null 2>&1; then \
			echo ""; \
			echo "❌ Missing gcc-aarch64-linux-gnu!"; \
			echo ""; \
			echo "Please install dependencies first:"; \
			echo "  sudo apt-get update && sudo apt-get install -y --no-install-recommends \\"; \
			echo "    build-essential pkg-config libglib2.0-dev libssl-dev curl wget file \\"; \
			echo "    gcc-aarch64-linux-gnu"; \
			echo ""; \
			echo "Then run: make package-linux"; \
			exit 1; \
		fi; \
	fi

# Build and package Linux x86_64 binary
package-linux-x86: check-linux-deps
	@mkdir -p dist
	@echo "=== Building Linux x86_64 ==="
	cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli
	@tar czf dist/nuwax-cli-linux-amd64.tar.gz -C target/x86_64-unknown-linux-gnu/release nuwax-cli
	@echo "✅ dist/nuwax-cli-linux-amd64.tar.gz"

# Build and package Linux ARM64 binary
package-linux-arm64: check-linux-deps
	@mkdir -p dist
	@echo "=== Building Linux ARM64 ==="
	cargo build --release --target aarch64-unknown-linux-gnu -p nuwax-cli
	@tar czf dist/nuwax-cli-linux-arm64.tar.gz -C target/aarch64-unknown-linux-gnu/release nuwax-cli
	@echo "✅ dist/nuwax-cli-linux-arm64.tar.gz"

# Build and package Linux (x86_64 + arm64)
# Note: builds sequentially to avoid cargo build lock contention
package-linux: check-linux-deps
	@mkdir -p dist
	@echo "=== Building Linux x86_64 ==="
	cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli
	@tar czf dist/nuwax-cli-linux-amd64.tar.gz -C target/x86_64-unknown-linux-gnu/release nuwax-cli
	@echo "✅ dist/nuwax-cli-linux-amd64.tar.gz"
	@echo "=== Building Linux ARM64 ==="
	cargo build --release --target aarch64-unknown-linux-gnu -p nuwax-cli
	@tar czf dist/nuwax-cli-linux-arm64.tar.gz -C target/aarch64-unknown-linux-gnu/release nuwax-cli
	@echo "✅ dist/nuwax-cli-linux-arm64.tar.gz"
	@echo ""
	@echo "=== All Linux packages built ==="
	@ls -la dist/nuwax-cli-linux-*.tar.gz
