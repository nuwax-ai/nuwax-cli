# Ubuntu 22.04 based Rust build environment for nuwax-cli
# 确保 glibc 兼容性: 从源码编译 Rust，确保链接到 Ubuntu 22.04 系统的 libstdc++

FROM ubuntu:22.04

# 设置环境
ENV DEBIAN_FRONTEND=noninteractive
ENV DEBCONF_NONINTERACTIVE_SEEN=true

# 安装构建依赖
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

# 从源码编译安装 Rust (不使用预编译版本，确保链接到系统库)
# 克隆 Rust 源码
RUN git clone --depth 1 --branch stable https://github.com/rust-lang/rust.git /rust-src

# 配置并编译 Rust
# 注意：这会编译很长时间（约 30-60 分钟）
WORKDIR /rust-src
RUN ./configure \
    --prefix=/opt/rust \
    --mandir=/share/man \
    --datadir=/share/rust \
    --sysconfdir=/etc/rust \
    --disable-docs \
    --optimize=0 \
    && make -j$(nproc)

# 安装 Rust
RUN make install

# 设置 PATH
ENV PATH="/opt/rust/bin:${PATH}"
ENV CARGO_HOME=/root/.cargo
ENV LD_LIBRARY_PATH="/opt/rust/lib:${LD_LIBRARY_PATH}"

# 验证 Rust 版本
RUN rustc --version && cargo --version

# 清理源码以减小镜像体积
WORKDIR /
RUN rm -rf /rust-src

# 复制源码
WORKDIR /workspace
COPY . .

# 添加 x86_64 target 并构建
RUN rustup target add x86_64-unknown-linux-gnu && \
    cargo build --release --target x86_64-unknown-linux-gnu -p nuwax-cli

# 默认输出目录
CMD ["tail", "-f", "/dev/null"]