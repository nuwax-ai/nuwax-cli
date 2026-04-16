# Ubuntu 20.04 based Rust build environment for nuwax-cli
# 使用 Ubuntu 20.04 构建，确保与 Ubuntu 20.04+ 系统兼容

FROM ubuntu:20.04

# 设置环境
ENV DEBIAN_FRONTEND=noninteractive
ENV DEBCONF_NONINTERACTIVE_SEEN=true

# 安装构建依赖
# gcc-multilib 仅在 x86_64 架构时安装
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libglib2.0-dev \
    libssl-dev \
    curl \
    file \
    wget \
    git \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# 使用 rustup 官方安装 Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable

# 设置 PATH
ENV PATH="/root/.cargo/bin:${PATH}"
ENV CARGO_HOME=/root/.cargo
ENV RUSTUP_HOME=/root/.rustup

# 验证 Rust 版本
RUN rustc --version && cargo --version

# 复制源码
WORKDIR /workspace
COPY . .

# 动态检测架构并添加对应的 Rust target
RUN ARCH=$(uname -m) && \
    if [ "$ARCH" = "x86_64" ]; then \
        rustup target add x86_64-unknown-linux-gnu; \
        cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli; \
    elif [ "$ARCH" = "aarch64" ]; then \
        rustup target add aarch64-unknown-linux-gnu; \
        cargo build --release --target aarch64-unknown-linux-gnu -p nuwax-cli; \
    else \
        echo "Unsupported architecture: $ARCH"; \
        exit 1; \
    fi && \
    mkdir -p /output && \
    cp target/*/release/nuwax-cli /output/