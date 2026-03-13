# 贡献指南 (Contributing)

感谢您有兴趣为 ZhcLaw 项目做出贡献！本文档提供了开发指南和 PR 规范。

## 开发指南

### 环境要求

- Rust 1.70+ (通过 [rustup](https://rustup.rs/) 安装)
- Cargo
- Git

### 快速开发环境设置

```bash
# 1. 克隆仓库
git clone https://github.com/yourusername/zhclaw.git
cd zhclaw

# 2. 安装 Rust（如果尚未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 3. 安装依赖
cargo build

# 4. 运行测试确保环境正确
cargo test
```

### 项目结构

```
src/
├── main.rs                 # 应用入口
├── config.rs               # 配置管理
├── lib.rs                  # 库导出
├── router.rs               # 消息路由
├── channel/
│   ├── mod.rs              # Channel 模块入口
│   ├── types.rs            # 通用类型定义
│   └── telegram.rs         # Telegram 适配器实现
├── executor/
│   ├── mod.rs              # Executor 模块，包含 AgentExecutor
│   └── process_registry.rs # 进程注册表
└── mcp/
    ├── mod.rs              # MCP Server 实现
    └── timer_manager.rs    # 定时器管理

tests/
├── executor_test.rs        # Executor 集成测试
├── mcp_test.rs             # MCP Server 集成测试
├── telegram_test.rs        # Telegram 适配器集成测试
└── e2e_test.rs             # 端到端集成测试
```

## PR 规范

### 开发流程

1. **创建 Issue**（如果还没有）
   - 清晰描述问题或功能需求
   - 参考 Issue 编号进行后续工作

2. **Fork 仓库**
   ```bash
   git clone https://github.com/yourusername/zhclaw.git
   cd zhclaw
   ```

3. **创建特性分支**
   ```bash
   git checkout -b feature/your-feature-name
   # 或bug修复
   git checkout -b fix/issue-number
   ```

4. **开发与测试**
   ```bash
   # 在开发过程中频繁运行测试
   cargo test
   
   # 检查代码质量
   cargo clippy
   
   # 格式化代码
   cargo fmt
   ```

5. **提交 Commit**
   - 遵循约定式提交 (Conventional Commits)
   - 格式: `<type>(<scope>): <subject>`
   - 例子:
     ```bash
     git commit -m "feat(executor): add support for custom timeout"
     git commit -m "fix(telegram): handle unicode message splitting correctly"
     git commit -m "docs(readme): update development guide"
     git commit -m "test(e2e): add integration tests for permission checking"
     ```

6. **推送并创建 PR**
   ```bash
   git push origin feature/your-feature-name
   ```
   然后在 GitHub 上创建 Pull Request

### Commit 类型

- `feat`: 新功能
- `fix`: bug 修复
- `docs`: 文档更新
- `style`: 代码格式（不影响功能）
- `refactor`: 代码重构
- `test`: 测试增加或修改
- `chore`: 构建工具、依赖更新等

### PR 检查清单

在提交 PR 前，请满足以下条件：

- [ ] 代码已经过单元测试和集成测试
- [ ] `cargo clippy` 没有警告
- [ ] `cargo fmt` 已运行（代码格式正确）
- [ ] 文档已更新（如有 API 变更）
- [ ] PR 描述清晰，包含相关 Issue 链接

### 代码质量要求

- **测试覆盖**: 新功能应包含相应的单元测试或集成测试
- **文档**: 公开 API 应有 doc 注释
- **格式**: 遵循 Rust 风格指南
- **性能**: 避免不必要的 clone，合理使用 Arc/RwLock

### 测试指南

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test executor_test

# 运行特定测试函数
cargo test test_executor_timeout

# 显示输出内容
cargo test -- --nocapture

# 单线程运行（便于调试）
cargo test -- --test-threads=1
```

### 常见开发任务

#### 添加新的命令行工具

1. 在 `src/executor/mod.rs` 中添加新的命令处理逻辑
2. 编写单元测试
3. 在 `tests/executor_test.rs` 中添加集成测试

#### 添加新的 Channel 适配器

1. 在 `src/channel/` 中创建新文件（如 `slack.rs`）
2. 实现 `ChannelAdapter` trait
3. 在 `src/channel/mod.rs` 中导出
4. 添加测试文件

#### 修改 MCP Server

1. 编辑 `src/mcp/mod.rs`
2. 更新 `handle_mcp_request` 函数
3. 在 `tests/mcp_test.rs` 中添加相应的测试

## 报告 Bug

在创建 Bug 报告时，请包含：

1. **系统信息**: 操作系统、Rust 版本等
2. **复现步骤**: 清晰的步骤说明如何重现问题
3. **期望行为**: 预期的结果是什么
4. **实际行为**: 实际发生了什么
5. **日志输出**: 相关的日志（设置 LOG_LEVEL=debug）
6. **代码示例**: 如果可能，提供最小化的复现代码

## 讨论与反馈

- 使用 GitHub Discussions 进行功能讨论
- 在 GitHub Issues 中报告 bug
- 在 PR 中进行代码审查讨论

## 许可证

通过提交 PR，您同意将您的代码贡献在 MIT 许可证下进行发布。

## 致谢

感谢所有为 ZhcLaw 做出贡献的开发者！
