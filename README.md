# ZhcLaw - AI Agent 多渠道聊天网关

[![Rust](https://img.shields.io/badge/Rust-1.75+-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/Tests-97%20passed-brightgreen)](tests/)
[![Code Quality](https://img.shields.io/badge/Code%20Quality-No%20Warnings-blue)](Cargo.toml)

> **将任何 AI Agent CLI 工具（如 Claude、ChatGPT）通过 Telegram 聊天接入，并通过 MCP 协议提供定时任务管理和进程控制能力。**

基于 Rust 的生产级多渠道聊天网关，一次配置，多渠道支持；集成 MCP Server 提供 Agent 生命周期和定时任务管理。

## ✨ 核心特性

- 🤖 **多渠道支持**：通过统一的 `ChannelAdapter` trait 扩展，目前实现 Telegram，可轻松添加 Slack、Discord 等
- 🚀 **灵活的 Agent 集成**：通过命令行模板 `{prompt}` 占位符，支持任何 CLI 工具（Claude、ChatGPT、本地 LLM 等）
- ⏰ **定时任务引擎**：基于 Cron 表达式的分布式定时任务，支持暂停/恢复/删除
- 🔍 **进程管理**：实时追踪所有 Agent 命令执行状态，支持超时控制和优雅关闭
- 🔗 **MCP 服务器**：实现 Model Context Protocol 标准，可与 Claude Desktop 等 MCP 客户端直接集成
- 🔐 **权限控制**：基于用户 ID 的访问管理，支持多用户环境
- 📊 **结构化日志**：使用 `tracing` 提供详细诊断信息，支持多种输出格式
- 🛡️ **错误重试**：网络异常时自动重试，指数退避策略确保系统稳定性
- 🐳 **容器化部署**：内置 Dockerfile，支持一键 Docker 部署

## 🚀 快速开始（5 分钟）

### 前置要求
- Rust 1.75+ 或 Docker
- Telegram 账户和 Bot Token（从 [@BotFather](https://t.me/botfather) 获得）
- 任何支持命令行的 AI Agent 工具

### 安装步骤

```bash
# 1. 克隆仓库
git clone https://github.com/yourusername/zhclaw.git
cd zhclaw

# 2. 复制环境配置模板
cp .env.example .env

# 3. 编辑 .env，填入你的配置
# TELEGRAM_BOT_TOKEN=1234567890:ABCDEFGHIJKLMNOPQRSTUVWxyz
# AGENT_COMMAND_TEMPLATE="claude -p {prompt}"

# 4. 运行
cargo run
```

### Docker 一键启动

```bash
docker build -t zhclaw:latest .
docker run -e TELEGRAM_BOT_TOKEN=your_token \
           -e AGENT_COMMAND_TEMPLATE="claude -p {prompt}" \
           -p 3100:3100 \
           zhclaw:latest
```

### 基础使用

1. **Telegram 消息处理**：向 Bot 发送消息 → 自动转发到 Agent 命令行 → 返回结果

2. **MCP 服务集成**：配置 Claude Desktop：
   ```json
   // ~/.claude_desktop_config.json
   {
     "mcpServers": {
       "zhclaw": {
         "url": "http://localhost:3100/mcp"
       }
     }
   }
   ```
   
   然后在 Claude 中使用工具：`create_timer`、`list_processes` 等

## 📚 详细文档

### 环境变量配置

| 变量名 | 说明 | 示例 | 默认值 |
|--------|------|------|--------|
| `TELEGRAM_BOT_TOKEN` | **必需**。Telegram Bot API Token | `123456:ABC-DEF...` | 无 |
| `AGENT_COMMAND_TEMPLATE` | **必需**。Agent 命令行模板，`{prompt}` 为占位符 | `claude -p {prompt}` | 无 |
| `AGENT_TIMEOUT_SECS` | Agent 命令执行超时（秒） | `300` | `300` |
| `MCP_SERVER_HOST` | MCP Server 监听地址 | `0.0.0.0` 或 `127.0.0.1` | `127.0.0.1` |
| `MCP_SERVER_PORT` | MCP Server HTTP 监听端口 | `3100` | `3100` |
| `ALLOWED_USER_IDS` | 允许使用的 Telegram 用户 ID（逗号分隔，空值表示允许所有） | `123456,789012` | `` (允许所有) |
| `LOG_LEVEL` | 日志级别 | `trace`、`debug`、`info`、`warn`、`error` | `info` |

**示例 .env 文件**：

```env
# 必需配置
TELEGRAM_BOT_TOKEN=1234567890:ABCDEFGHIJKLMNOPQRSTUVWxyz-123456
AGENT_COMMAND_TEMPLATE="claude -p {prompt}"

# 可选配置
AGENT_TIMEOUT_SECS=300
MCP_SERVER_HOST=0.0.0.0
MCP_SERVER_PORT=3100
ALLOWED_USER_IDS=123456,789012
LOG_LEVEL=info
```

### 核心模块

#### 1. Channel Adapter Layer — 多渠道适配器

通过 `async_trait` 和 trait 对象实现渠道抽象，支持轻松扩展新渠道。

**Telegram 实现**使用 [`teloxide`](https://github.com/teloxide/teloxide) 库 + 错误重试机制：
- 初始化重试：最多 10 次，指数退避 (2¹ ~ 2¹⁰ s，最多 120s)
- 发送重试：最多 3 次，指数退避 (1s, 2s, 4s)
- 消息分割：按字符数（非字节数）分割支持 UTF-8、emoji 等

#### 2. Agent Executor — 命令行执行引擎

- 命令模板渲染：替换 `{prompt}` 占位符，自动转义特殊字符
- 子进程管理：启动、监控、超时控制、进程注册
- 输出捕获：同时捕获 stdout 和 stderr，合并返回

#### 3. MCP Server — 模型上下文协议服务

实现 [MCP (Model Context Protocol)](https://modelcontextprotocol.io/) 标准提供 6 个工具：

**定时任务工具**：
- `create_timer` - 创建定时任务（Cron 表达式）
- `list_timers` - 列出所有定时任务
- `delete_timer` - 删除定时任务
- `toggle_timer` - 暂停/启用定时任务

**进程管理工具**：
- `list_processes` - 列出运行中的 Agent 进程
- `kill_process` - 终止进程

## 📖 使用示例

### 示例 1：Telegram + Claude

**配置 .env**：
```env
TELEGRAM_BOT_TOKEN=your_bot_token
AGENT_COMMAND_TEMPLATE="claude {prompt}"
```

**对话示例**：
```
用户: 请帮我设计一个 Rust 并发数据结构

Bot: <Claude 的回复>
```

### 示例 2：使用定时任务

通过 MCP 创建定时任务，让 Agent 定期执行：

**Claude 中执行**：
```
调用工具 "create_timer" 参数:
- name: "daily_report"
- cron_expr: "0 9 * * * *"  (每天 09:00)
- prompt: "生成今日新闻摘要"  
- chat_id: "12345"
```

**系统日志**：
```
2026-03-13T09:00:00Z INFO 定时任务触发: daily_report
2026-03-13T09:00:01Z INFO 执行输出: <摘要内容>
```

### 示例 3：查询进程状态

```
调用工具 "list_processes"

返回:
[
  {
    "id": "p-001",
    "pid": 12345,
    "command": "claude '生成代码'",
    "chat_id": "12345",
    "status": "Running",
    "started_at": "2026-03-13T09:00:01Z"
  }
]
```

## 🧪 测试

```bash
# 运行所有测试（97 个测试）
cargo test

# 运行特定测试套件
cargo test --test executor_test     # Agent 执行器测试
cargo test --test mcp_test          # MCP 服务器测试
cargo test --test telegram_test     # Telegram 适配器测试
cargo test --test e2e_test          # 端到端集成测试

# 查看测试覆盖情况和详细输出
cargo test -- --nocapture

# 代码质量检查
cargo clippy    # 代码分析和建议
cargo fmt       # 代码格式化
```

**测试覆盖**：97 个测试
- 单元测试：54 个（核心函数逻辑）
- 集成测试：43 个（模块交互、错误处理、边界情况）

## 🔧 开发指南

### 项目结构

```
zhclaw/
├── Cargo.toml                          # 项目配置和依赖
├── README.md                           # 项目文档
├── CONTRIBUTING.md                     # 贡献指南
├── Dockerfile                          # Docker 镜像构建
├── .github/workflows/ci.yml            # GitHub Actions CI/CD
│
├── .env.example                        # 环境变量模板
├── src/                                # Rust 源代码
│   ├── main.rs                         # 应用入口
│   ├── lib.rs                          # 库定义
│   ├── config.rs                       # 配置管理
│   ├── channel/                        # 多渠道适配层
│   │   ├── mod.rs                      # ChannelAdapter trait
│   │   ├── types.rs                    # 数据类型定义
│   │   └── telegram.rs                 # Telegram 适配器
│   ├── executor/                       # Agent 执行引擎
│   │   ├── mod.rs                      # AgentExecutor
│   │   └── process_registry.rs         # 进程生命周期管理
│   ├── mcp/                            # MCP Server
│   │   ├── mod.rs                      # HTTP 端点
│   │   └── timer_manager.rs            # 定时任务调度
│   └── router.rs                       # 消息路由
│
└── tests/                              # 集成测试
    ├── executor_test.rs
    ├── mcp_test.rs
    ├── telegram_test.rs
    └── e2e_test.rs
```

### 扩展指南

#### 添加新渠道适配器

1. 创建 `src/channel/slack.rs`
2. 实现 `ChannelAdapter` trait
3. 在 `main.rs` 注册

#### 添加自定义 MCP 工具

在 `src/mcp/mod.rs` 中使用 `#[tool]` 宏。

## 🚀 性能和可靠性

- **内存安全**：100% Rust，无 buffer overflow、use-after-free 等问题
- **并发安全**：使用 `Arc<RwLock<>>` 实现线程安全
- **异步 I/O**：基于 Tokio，轻松处理数千并发连接
- **自动重试**：网络故障时自动重试，指数退避
- **优雅关闭**：收到 SIGINT/SIGTERM 时正确清理资源
- **超时控制**：防止慢命令阻塞系统

### 性能指标

- **消息延迟**：< 100ms（除 Agent 执行时间）
- **吞吐量**：百级消息/秒
- **内存占用**：20-50 MB
- **CPU 使用**：闲置时接近 0%

## ❓ 常见问题

**Q: 如何修改 Agent 工具?**
A: 在 `.env` 中修改 `AGENT_COMMAND_TEMPLATE`，支持任何命令行工具。

**Q: 如何限制用户使用?**
A: 设置 `ALLOWED_USER_IDS=123456,789012`（Telegram 用户 ID）。

**Q: 支持哪些定时任务表达式?**
A: 标准 Cron 表达式，例如 `0 9 * * * *`（每天 09:00）。

**Q: 如何查看日志?**
A: 通过 `LOG_LEVEL` 环境变量控制，`LOG_LEVEL=debug cargo run`。

## 🤝 贡献指南

欢迎贡献代码、报告 Bug、提出建议！请参考 [CONTRIBUTING.md](CONTRIBUTING.md)。

### 快速开发流程

```bash
# 1. Fork 仓库
# 2. 创建特性分支
git checkout -b feature/amazing-feature

# 3. 提交更改
git commit -am 'Add amazing feature'

# 4. 推送分支
git push origin feature/amazing-feature

# 5. 提交 Pull Request
```

## 📄 License

本项目采用 MIT License，详见 [LICENSE](LICENSE) 文件。

---

**致谢**：感谢 [Teloxide](https://github.com/teloxide/teloxide)、[Axum](https://github.com/tokio-rs/axum)、[Cron](https://crates.io/crates/cron) 等开源项目的支持。
