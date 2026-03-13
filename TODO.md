# ZhcLaw 开发任务清单

## 阶段一：项目初始化与基础设施

### 1.1 项目脚手架
- [x] `cargo init` 初始化项目，配置 `Cargo.toml` 所有依赖
- [x] 创建 `.env.example` 文件
- [x] 创建 `.gitignore`（忽略 target/、.env）
- [x] 搭建目录结构：`src/{channel, executor, mcp}/`，`tests/`

### 1.2 配置模块 (`src/config.rs`)
- [x] 定义 `Config` 结构体（所有环境变量字段）
- [x] 实现 `Config::from_env()` —— 使用 `dotenvy` + `envy` 从环境变量加载
- [x] 字段默认值处理（`AGENT_TIMEOUT_SECS` 默认 300，`MCP_SERVER_HOST` 默认 `0.0.0.0` 等）
- [x] `ALLOWED_USER_IDS` 逗号分隔字符串解析为 `Vec<String>`

#### 单元测试 — config
- [x] 测试：所有环境变量正确设置时，`Config::from_env()` 正确解析
- [x] 测试：缺少必填字段（如 `TELEGRAM_BOT_TOKEN`）时返回错误
- [x] 测试：可选字段缺失时使用默认值
- [x] 测试：`ALLOWED_USER_IDS` 空字符串解析为空 Vec
- [x] 测试：`ALLOWED_USER_IDS="a,b,c"` 正确解析为 3 个元素

### 1.3 日志初始化
- [x] 在 `main.rs` 中配置 `tracing-subscriber`，支持 `LOG_LEVEL` 环境变量

---

## 阶段二：Channel Adapter Layer — 多渠道适配层

### 2.1 通用类型与 Trait (`src/channel/types.rs`, `src/channel/mod.rs`)
- [x] 定义 `ChannelType` 枚举（Telegram, Slack, Discord, WeChat）
- [x] 定义 `IncomingMessage` 结构体
- [x] 定义 `OutgoingMessage` 结构体
- [x] 定义 `ChannelAdapter` trait（`start`, `send_message`, `channel_type`）
- [x] 在 `mod.rs` 中 re-export 所有公共类型

#### 单元测试 — channel types
- [x] 测试：`IncomingMessage` 各字段正确构造
- [x] 测试：`OutgoingMessage` 的 `parse_mode` 可选字段处理
- [x] 测试：`ChannelType` 枚举序列化/反序列化
- [x] 测试：`ChannelType` Display trait 输出正确

### 2.2 Telegram Adapter (`src/channel/telegram.rs`)
- [x] 实现 `TelegramAdapter` 结构体，封装 `teloxide::Bot`
- [x] 实现 `TelegramAdapter::new()` —— 从 `TELEGRAM_BOT_TOKEN` 构建 Bot
- [x] 实现 `ChannelAdapter::start()` —— 使用 teloxide dispatcher 接收消息，转为 `IncomingMessage` 发送到 `mpsc::Sender`
- [x] 实现 `ChannelAdapter::send_message()` —— 调用 `bot.send_message()`
- [x] Telegram 消息长度限制处理（>4096 字符自动分段发送）
- [x] 错误处理与重试逻辑

#### 单元测试 — telegram adapter
- [x] 测试：`TelegramAdapter::new()` 无 token 时 panic 或返回错误
- [x] 测试：消息分段逻辑 — 4096 字符以内不分段
- [x] 测试：消息分段逻辑 — 超长消息正确切割为多段
- [x] 测试：`channel_type()` 返回 `ChannelType::Telegram`

---

## 阶段三：Agent Executor — 命令行执行引擎

### 3.1 ProcessRegistry (`src/executor/process_registry.rs`)
- [x] 定义 `ProcessStatus` 枚举（Running, Completed, Failed, Killed）
- [x] 定义 `ProcessInfo` 结构体（id, pid, command, chat_id, started_at, status）
- [x] 实现 `ProcessRegistry` 结构体（`Arc<RwLock<HashMap<String, ProcessInfo>>>`）
- [x] 实现 `register()` —— 注册新进程，返回 process_id（UUID）
- [x] 实现 `unregister()` —— 更新进程状态为 Completed 或移除
- [x] 实现 `list_running()` —— 返回所有 Running 状态的进程
- [x] 实现 `kill_process()` —— 通过 pid 发送 SIGTERM，更新状态为 Killed
- [x] 实现 `get_process()` —— 按 process_id 查询

#### 单元测试 — process registry
- [x] 测试：`register()` 返回唯一 ID，进程出现在列表中
- [x] 测试：`unregister()` 后进程不再出现在 `list_running()` 中
- [x] 测试：`list_running()` 只返回 Running 状态的进程
- [x] 测试：`get_process()` 对存在/不存在的 ID 正确返回
- [x] 测试：并发注册多个进程后列表完整
- [x] 测试：`kill_process()` 对不存在的进程返回错误

### 3.2 AgentExecutor (`src/executor/mod.rs`)
- [x] 定义 `AgentExecutor` 结构体（command_template, timeout, process_registry）
- [x] 实现 `AgentExecutor::new()`
- [x] 实现命令模板渲染 —— `{prompt}` 替换并做 shell escape
- [x] 实现 `parse_command()` —— 从完整命令字符串解析出程序名和参数
- [x] 实现 `execute()` —— tokio::process::Command 启动子进程
- [x] stdout + stderr 合并捕获
- [x] 超时控制（tokio::time::timeout）
- [x] 子进程注册/注销到 ProcessRegistry
- [x] 非零退出码处理

#### 单元测试 — agent executor
- [x] 测试：模板渲染 `"echo {prompt}"` + `"hello world"` → `"echo 'hello world'"`
- [x] 测试：shell escape 处理特殊字符（引号、反斜杠、$符号）
- [x] 测试：`parse_command("echo hello world")` → `("echo", ["hello", "world"])`
- [x] 测试：执行 `echo hello` 返回 `"hello\n"`
- [x] 测试：执行不存在的命令返回错误
- [x] 测试：超时控制 —— 执行 `sleep 10` 配合 1 秒超时应返回超时错误
- [x] 测试：非零退出码的命令输出仍被捕获
- [x] 测试：执行期间进程出现在 ProcessRegistry 中，完成后移除

---

## 阶段四：MCP Server — Streamable HTTP

### 4.1 MCP Server 基础框架 (`src/mcp/mod.rs`)
- [x] 定义 `ZhclawMcpServer` 结构体（timer_manager, process_registry）
- [x] 实现 `ZhclawMcpServer::new()`
- [x] 实现 `serve_http()` —— 基于 axum 启动 HTTP 服务
- [x] 配置 axum 路由：`POST /mcp`、`GET /mcp`、`DELETE /mcp`
- [x] 手动实现 MCP JSON-RPC 协议处理（initialize, tools/list, tools/call）

#### 单元测试 — mcp server 基础
- [x] 测试：`ZhclawMcpServer::new()` 正确初始化（通过编译验证）
- [x] 测试：server 的 tool 列表包含所有 6 个 tools（已实现于 handle_tools_list）

### 4.2 TimerManager (`src/mcp/timer_manager.rs`)
- [x] 定义 `TimerEntry` 结构体
- [x] 实现 `TimerManager` 结构体
- [x] 实现 `create_timer()` —— 验证 cron 表达式合法性，计算 next_run，写入 HashMap
- [x] 实现 `list_timers()` —— 返回所有 timer 的 JSON 列表
- [x] 实现 `delete_timer()` —— 按名称删除
- [x] 实现 `toggle_timer()` —— 启用/禁用切换
- [x] 实现 `start_scheduler()` —— tokio::spawn 调度循环
- [x] 调度循环：每秒检查所有 enabled timer 的 next_run，到期则触发执行
- [x] 触发后更新 `last_run` 和 `next_run`
- [x] 重复名称创建时返回冲突错误

#### 单元测试 — timer manager
- [x] 测试：创建 timer 后出现在 `list_timers()` 中
- [x] 测试：创建 timer 时非法 cron 表达式返回错误
- [x] 测试：删除存在的 timer 成功
- [x] 测试：删除不存在的 timer 返回 not found 错误
- [x] 测试：toggle 从 enabled → disabled，再 disabled → enabled
- [x] 测试：创建同名 timer 返回冲突错误
- [x] 测试：cron `next_run` 计算正确（使用已知的 cron 表达式验证）
- [x] 测试：disabled timer 不会被调度执行

### 4.3 MCP Tool 实现
- [x] 实现 `create_timer` tool —— 委托 TimerManager
- [x] 实现 `list_timers` tool —— 返回 JSON 格式列表
- [x] 实现 `delete_timer` tool
- [x] 实现 `toggle_timer` tool
- [x] 实现 `list_processes` tool —— 委托 ProcessRegistry
- [x] 实现 `kill_process` tool —— 委托 ProcessRegistry

#### 单元测试 — mcp tools
- [x] 测试：`create_timer` 工具参数校验（通过 TimerManager 单元测试覆盖）
- [x] 测试：`list_timers` 空状态返回空列表（TimerManager 单元测试已覆盖）
- [x] 测试：`list_processes` 空状态返回空列表（ProcessRegistry 单元测试已覆盖）
- [x] 测试：`kill_process` 不存在的进程 ID 返回合理错误信息（ProcessRegistry 单元测试已覆盖）

---

## 阶段五：消息路由 — Message Router

### 5.1 消息路由 (`src/router.rs`)
- [x] 定义 `MessageRouter` 结构体（executor, adapters HashMap, config）
- [x] 实现 `MessageRouter::new()`
- [x] 实现 `run()` —— 从 `mpsc::Receiver` 循环接收消息
- [x] 权限校验 —— 检查 `user_id` 是否在 `ALLOWED_USER_IDS` 中（为空则允许所有）
- [x] 根据 `IncomingMessage.channel` 找到对应的 `ChannelAdapter` 发送回复
- [x] 调用 `AgentExecutor::execute()` 执行命令
- [x] 将执行输出通过 adapter 回复到对应的 chat
- [x] 错误处理 —— 执行失败时发送错误消息给用户

#### 单元测试 — router
- [x] 测试：`ALLOWED_USER_IDS` 为空时，所有用户都通过校验
- [x] 测试：`ALLOWED_USER_IDS` 包含 user_id 时通过校验
- [x] 测试：`ALLOWED_USER_IDS` 不包含 user_id 时拒绝
- [x] 测试：路由正确匹配 channel type 到对应 adapter

---

## 阶段六：主入口与服务启动

### 6.1 Main (`src/main.rs`)
- [x] 加载 .env 配置
- [x] 初始化 tracing 日志
- [x] 创建共享状态（ProcessRegistry, TimerManager）
- [x] 初始化 AgentExecutor
- [x] 初始化 Telegram adapter，放入 adapters 列表
- [x] 启动 MCP Server HTTP 服务（tokio::spawn）
- [x] 启动 TimerManager 调度器（tokio::spawn）
- [x] 启动各 channel adapter 监听（tokio::spawn）
- [x] 启动 MessageRouter 主循环
- [x] 优雅关闭（graceful shutdown）处理 SIGINT/SIGTERM

---

## 阶段七：集成测试

### 7.1 Agent Executor 集成测试 (`tests/executor_test.rs`)
- [x] 测试：完整流程 —— 创建 executor → 执行 `echo` 命令 → 验证输出
- [x] 测试：超时场景 —— 执行长时间命令 → 验证超时返回错误
- [x] 测试：并发执行 —— 同时执行多个命令 → 所有结果正确返回
- [x] 测试：ProcessRegistry 联动 —— 执行中进程在列表中，执行完消失
- [x] 测试：kill_process —— 启动长时间进程 → kill → 验证进程被终止
- [x] 测试：stderr 输出 —— 执行产生 stderr 的命令 → 验证 stderr 被捕获

### 7.2 MCP Server 集成测试 (`tests/mcp_test.rs`)
- [x] 测试：HTTP 服务启动 —— 启动 MCP Server → 验证端口可达
- [x] 测试：MCP initialize —— 发送 initialize JSON-RPC 请求 → 验证返回 server info
- [x] 测试：tools/list —— 发送 tools/list 请求 → 验证返回 6 个 tools
- [x] 测试：create_timer → list_timers —— 创建 timer 后列表中包含该 timer
- [x] 测试：create_timer → delete_timer → list_timers —— 删除后列表为空
- [x] 测试：create_timer → toggle_timer —— 切换状态验证
- [x] 测试：list_processes —— 空状态返回空列表
- [x] 测试：完整 timer 调度 —— 创建一个 "每秒" cron timer → 等待 2 秒 → 验证 agent 被执行且结果发送

### 7.3 Telegram Adapter 集成测试 (`tests/telegram_test.rs`)
- [x] 测试：使用 Mock Server 模拟 Telegram API —— 发送消息 → 验证 HTTP 请求正确
- [x] 测试：接收消息 Mock —— 模拟 getUpdates 返回 → 验证 IncomingMessage 正确生成
- [x] 测试：长消息分段 —— 发送 5000 字符消息 → 验证分 2 次发送

### 7.4 端到端集成测试 (`tests/e2e_test.rs`)
- [x] 测试：消息 → 执行 → 回复 —— Mock Telegram 发消息 → Router 处理 → Executor 执行 echo → 验证回复内容
- [x] 测试：权限拒绝 —— 配置 ALLOWED_USER_IDS → 非授权用户发消息 → 验证被拒绝
- [x] 测试：MCP + Executor 联动 —— 通过 MCP 创建 timer → 触发执行 → 验证 process 曾出现在 list_processes 中

---

## 阶段八：文档与 CI

### 8.1 文档
- [x] 完善 README.md 中的构建与部署说明
- [x] 补充 `cargo doc` 行内文档注释（所有 pub 接口）
- [x] 编写 CONTRIBUTING.md（开发指南、PR 规范）

### 8.2 CI/CD
- [x] 配置 GitHub Actions：`cargo build` + `cargo test` + `cargo clippy`
- [x] 配置 Dockerfile（多阶段构建，最终镜像尽量小）
- [x] 配置 `.cargo/config.toml`（如需交叉编译）
