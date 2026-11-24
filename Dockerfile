# Debian 10 (buster) based Rust build environment for nuwax-cli
# 使用 Debian 10 构建，把产物的 glibc 基线钉在 2.28：
#   可在 UOS、Deepin 20、Debian 10、CentOS/RHEL 8 及更新的系统上运行（要求 GLIBC 2.28+）
#   CentOS 7 / RHEL 7（glibc 2.17）不支持

FROM debian:10

# 设置环境
ENV DEBIAN_FRONTEND=noninteractive
ENV DEBCONF_NONINTERACTIVE_SEEN=true

# 安装构建依赖
# buster 已停止维护，软件包只保留在归档源上；归档的 Release 文件已过期，需要关闭有效期检查。
# 基础镜像里没有 CA 证书，rustup 走 HTTPS 下载需要 ca-certificates。
RUN sed -i 's|http://deb.debian.org/debian-security|http://archive.debian.org/debian-security|; s|http://deb.debian.org/debian|http://archive.debian.org/debian|' /etc/apt/sources.list \
    && sed -i '/buster-updates/d' /etc/apt/sources.list \
    && echo 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99archive \
    && apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libglib2.0-dev \
    libssl-dev \
    curl \
    file \
    wget \
    git \
    cmake \
    ca-certificates \
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