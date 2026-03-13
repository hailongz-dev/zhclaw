# 多阶段构建：编译和运行时分离
FROM rust:1.75 as builder

WORKDIR /app

# 复制 Cargo 文件
COPY Cargo.toml Cargo.lock* ./

# 如有必要，创建 dummy 项目以缓存依赖
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>&1 | grep -v "warning" || true

# 删除 dummy 源代码
RUN rm -rf src

# 复制实际源代码
COPY src ./src

# 构建应用
RUN cargo build --release

# 最终运行时镜像（Alpine Linux 以减少大小）
FROM debian:bookworm-slim

WORKDIR /app

# 安装必要的运行时依赖
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 从 builder 阶段复制编译后的二进制文件
COPY --from=builder /app/target/release/zhclaw /usr/local/bin/zhclaw

# 设置环境变量默认值
ENV LOG_LEVEL=info
ENV MCP_SERVER_HOST=0.0.0.0
ENV MCP_SERVER_PORT=3000
ENV AGENT_TIMEOUT_SECS=300

# 暴露 MCP Server 端口
EXPOSE 3000

# 健康检查
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:${MCP_SERVER_PORT}/mcp || exit 1

# 启动应用
CMD ["zhclaw"]
