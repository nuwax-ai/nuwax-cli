.PHONY: build release dev clean test fmt clippy check help

# Default target
help:
	@echo "Nuwax CLI Makefile"
	@echo ""
	@echo "Usage:"
	@echo "  make build      - Build nuwax-cli (release)"
	@echo "  make release    - Build nuwax-cli (release)"
	@echo "  make dev        - Build nuwax-cli (debug)"
	@echo "  make clean      - Clean build artifacts"
	@echo "  make test       - Run all tests"
	@echo "  make fmt        - Format code"
	@echo "  make clippy     - Run clippy linter"
	@echo "  make check      - Check workspace"

# Build release version
build release:
	cargo build --release -p nuwax-cli

# Build debug version
dev:
	cargo build -p nuwax-cli

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
