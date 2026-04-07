# Ubuntu 22.04 based Rust build environment for nuwax-cli

FROM ubuntu:22.04

# 设置环境
ENV DEBIAN_FRONTEND=noninteractive
ENV DEBCONF_NONINTERACTIVE_SEEN=true

# 安装构建依赖（包括交叉编译工具链）
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
    gcc-multilib \
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

# 添加 x86_64 target 并交叉编译
RUN rustup target add x86_64-unknown-linux-gnu && \
    cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli && \
    mkdir -p /output && \
    cp target/x86_64-unknown-linux-gnu/release/nuwax-cli /output/