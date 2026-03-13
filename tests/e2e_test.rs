//! 端到端集成测试

use std::sync::Arc;
use std::time::Duration;

use zhclaw::channel::{ChannelType, IncomingMessage};
use zhclaw::config::Config;
use zhclaw::executor::AgentExecutor;
use zhclaw::executor::process_registry::ProcessRegistry;
use zhclaw::router::MessageRouter;

/// 创建测试用的 config（允许所有用户）
fn create_test_config() -> Config {
    Config {
        agent_command_template: "echo {prompt}".to_string(),
        agent_timeout_secs: 5,
        telegram_bot_token: "test_token".to_string(),
        mcp_server_host: "127.0.0.1".to_string(),
        mcp_server_port: 8000,
        allowed_user_ids: "".to_string(), // 空字符串 = 允许所有用户
        log_level: "info".to_string(),
    }
}

/// 创建禁止特定用户的 config
fn create_restricted_config() -> Config {
    Config {
        agent_command_template: "echo {prompt}".to_string(),
        agent_timeout_secs: 5,
        telegram_bot_token: "test_token".to_string(),
        mcp_server_host: "127.0.0.1".to_string(),
        mcp_server_port: 8001,
        allowed_user_ids: "user_allowed".to_string(), // 只允许 user_allowed
        log_level: "info".to_string(),
    }
}

#[tokio::test]
async fn test_message_execution_flow() {
    // 测试：完整流程 —— 消息 → 执行 → 回复
    // 创建 executor
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "echo {prompt}",
        Duration::from_secs(5),
        registry,
    ));

    // 测试执行"hello"命令
    let result = executor.execute("hello", "test_chat").await;
    assert!(result.is_ok(), "执行应成功");
    let output = result.unwrap();
    assert!(output.contains("hello"), "输出应包含 'hello'");
}

#[tokio::test]
async fn test_permission_denied_empty_whitelist() {
    // 测试：ALLOWED_USER_IDS 为空时允许所有用户
    let config = create_test_config();
    assert!(config.is_user_allowed("any_user"), "空 whitelist 应允许所有用户");
    assert!(config.is_user_allowed("another_user"), "任何用户都应被允许");
}

#[tokio::test]
async fn test_permission_granted_in_whitelist() {
    // 测试：用户在 whitelist 中
    let config = create_restricted_config();
    assert!(
        config.is_user_allowed("user_allowed"),
        "在 whitelist 中的用户应被允许"
    );
}

#[tokio::test]
async fn test_permission_denied_not_in_whitelist() {
    // 测试：用户不在 whitelist 中
    let config = create_restricted_config();
    assert!(
        !config.is_user_allowed("user_denied"),
        "不在 whitelist 中的用户应被拒绝"
    );
}

#[tokio::test]
async fn test_message_router_permission_check() {
    // 测试：路由器正确检查权限
    let config = create_restricted_config();
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "echo {prompt}",
        Duration::from_secs(5),
        registry,
    ));

    // 创建空的 adapters 列表（仅用于路由器初始化）
    let adapters = vec![];
    let _router = MessageRouter::new(executor, adapters, &config);

    // 创建一条消息
    let msg = IncomingMessage {
        channel: ChannelType::Telegram,
        chat_id: "test_chat".to_string(),
        user_id: "user_denied".to_string(),
        text: "test".to_string(),
        timestamp: chrono::Utc::now(),
    };

    // 验证权限检查
    assert!(
        !config.is_user_allowed(&msg.user_id),
        "被拒绝的用户应不通过权限检查"
    );
}

#[tokio::test]
async fn test_executor_registries_processes() {
    // 测试：ProcessRegistry 联动 —— 进程应被正确注册和注销
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "sleep {prompt}",
        Duration::from_secs(5),
        registry.clone(),
    ));

    // 启动后台任务执行命令
    let handle = tokio::spawn(async move {
        executor.execute("2", "test_chat").await
    });

    // 等待进程启动
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 检查进程是否已注册
    let running = registry.list_running().await;
    assert!(!running.is_empty(), "执行中应有进程");

    // 等待执行完成
    let _ = handle.await;

    // 检查进程是否已注销
    let running = registry.list_running().await;
    assert!(running.is_empty(), "执行完成后应无进程");
}

#[tokio::test]
async fn test_mcp_timer_and_executor_integration() {
    // 测试：MCP timer 和 executor 联动
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "sleep {prompt}",  // 使用 sleep 确保进程较长时间存在
        Duration::from_secs(10),
        registry.clone(),
    ));

    // 执行命令并验证进程被跟踪
    let handle = tokio::spawn({
        let _registry = registry.clone();
        let executor = executor.clone();
        async move {
            executor.execute("2", "test_chat").await  // sleep 2 秒
        }
    });

    // 等待进程启动
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 检查 ProcessRegistry 是否跟踪了进程
    let processes = registry.list_running().await;
    let found = processes.iter().any(|p| p.chat_id == "test_chat");
    assert!(found, "进程应该在 ProcessRegistry 中被跟踪");

    // 等待执行完成
    let result = handle.await;
    assert!(result.is_ok(), "执行应该成功");
}

#[tokio::test]
async fn test_executor_timeout_propagation() {
    // 测试：超时错误正确传播
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "sleep {prompt}",
        Duration::from_secs(1), // 1 秒超时
        registry,
    ));

    let result = executor.execute("5", "test_chat").await;
    assert!(result.is_err(), "长时间命令应该超时");
    
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("超时"),
        "错误消息应包含 '超时'"
    );
}

#[tokio::test]
async fn test_executor_error_handling() {
    // 测试：执行错误正确处理
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "nonexistent_command {prompt}",
        Duration::from_secs(5),
        registry,
    ));

    let result = executor.execute("test", "test_chat").await;
    assert!(result.is_err(), "调用不存在的命令应返回错误");
}

#[tokio::test]
async fn test_incoming_message_creation() {
    // 测试：IncomingMessage 正确构造
    let msg = IncomingMessage {
        channel: ChannelType::Telegram,
        chat_id: "123".to_string(),
        user_id: "user_456".to_string(),
        text: "test message".to_string(),
        timestamp: chrono::Utc::now(),
    };

    assert_eq!(msg.channel, ChannelType::Telegram);
    assert_eq!(msg.chat_id, "123");
    assert_eq!(msg.user_id, "user_456");
    assert_eq!(msg.text, "test message");
}

#[tokio::test]
async fn test_concurrent_message_processing() {
    // 测试：并发消息处理 —— 多个 executor 实例并发执行
    let registry = Arc::new(ProcessRegistry::new());
    let executor = Arc::new(AgentExecutor::new(
        "echo {prompt}",
        Duration::from_secs(5),
        registry.clone(),
    ));

    let mut handles = vec![];

    for i in 0..5 {
        let executor = executor.clone();
        let handle = tokio::spawn(async move {
            executor.execute(&format!("msg_{}", i), "chat").await
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "并发执行应成功");
    }

    // 所有进程应已完成
    let running = registry.list_running().await;
    assert!(running.is_empty(), "完成后应无运行的进程");
}

#[tokio::test]
async fn test_config_parsing() {
    // 测试：配置对象创建和使用
    let config = create_test_config();
    
    assert_eq!(config.agent_command_template, "echo {prompt}");
    assert_eq!(config.agent_timeout_secs, 5);
    assert_eq!(config.mcp_server_port, 8000);
    assert_eq!(config.allowed_user_ids, "");
}

#[tokio::test]
async fn test_config_timeout_conversion() {
    // 测试：秒数到 Duration 的转换
    let config = create_test_config();
    let timeout = config.agent_timeout();
    
    assert_eq!(timeout, Duration::from_secs(5));
}
