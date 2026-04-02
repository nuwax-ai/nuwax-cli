# Ubuntu 22.04 based Rust build environment for nuwax-cli
# 确保 glibc 兼容性: 编译产物兼容 Ubuntu 22.04+

FROM ubuntu:22.04

# 设置环境
ENV DEBIAN_FRONTEND=noninteractive
ENV RUST_TARGET=x86_64-unknown-linux-gnu

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libglib2.0-dev \
    libssl-dev \
    curl \
    file \
    wget \
    && rm -rf /var/lib/apt/lists/*

# 安装 Rust 1.85+ 以支持 edition2024
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.85.0
ENV PATH="/root/.cargo/bin:${PATH}"
ENV PKG_CONFIG_PATH="/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig:/usr/lib/pkgconfig"

# 设置工作目录
WORKDIR /workspace

# 复制源码 (会在 docker build 时通过 --build-arg context=. 覆盖)
COPY . .

# 构建命令 (可通过 docker build --build-arg BUILD_CMD="..." 覆盖)
ARG BUILD_CMD="cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli"
RUN echo "Building with: ${BUILD_CMD}" && ${BUILD_CMD}

# 默认输出目录
CMD ["tail", "-f", "/dev/null"]
