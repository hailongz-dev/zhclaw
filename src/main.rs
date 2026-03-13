pub mod channel;
pub mod config;
pub mod executor;
pub mod mcp;
pub mod router;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::channel::telegram::TelegramAdapter;
use crate::channel::ChannelAdapter;
use crate::config::Config;
use crate::executor::process_registry::ProcessRegistry;
use crate::executor::AgentExecutor;
use crate::mcp::timer_manager::TimerManager;
use crate::mcp::ZhclawMcpServer;
use crate::router::MessageRouter;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 加载 .env
    dotenvy::dotenv().ok();

    // 2. 加载配置
    let config = Config::from_env()?;

    // 3. 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level)),
        )
        .init();

    info!("zhclaw 启动中...");

    // 4. 初始化共享状态
    let process_registry = Arc::new(ProcessRegistry::new());
    let timer_manager = Arc::new(TimerManager::new());

    // 5. 初始化 Agent Executor
    let executor = Arc::new(AgentExecutor::new(
        &config.agent_command_template,
        config.agent_timeout(),
        process_registry.clone(),
    ));

    // 6. 初始化渠道适配器
    let (msg_tx, msg_rx) = mpsc::channel(256);
    let telegram = Arc::new(TelegramAdapter::new(&config.telegram_bot_token));
    let adapters: Vec<Arc<dyn ChannelAdapter>> = vec![telegram.clone()];

    // 7. 启动 MCP Server (HTTP, 后台)
    let mcp_host = config.mcp_server_host.clone();
    let mcp_port = config.mcp_server_port;
    let mcp_timer_manager = timer_manager.clone();
    let mcp_process_registry = process_registry.clone();

    tokio::spawn(async move {
        let mcp_server = ZhclawMcpServer::new(
            mcp_timer_manager,
            mcp_process_registry,
        );
        let addr = format!("{}:{}", mcp_host, mcp_port);
        
        if let Err(e) = crate::mcp::serve_http(&addr, mcp_server).await {
            tracing::error!("MCP Server 启动失败: {}", e);
        }
    });

    // 8. 启动定时器调度
    let scheduler_executor = executor.clone();
    let scheduler_adapters = adapters.clone();
    timer_manager.start_scheduler(move |timer| {
        let exec = scheduler_executor.clone();
        let adapters = scheduler_adapters.clone();
        async move {
            info!("定时任务触发: {}", timer.name);
            match exec.execute(&timer.prompt, &timer.chat_id).await {
                Ok(output) => {
                    // 找到对应 adapter 发送结果
                    for adapter in &adapters {
                        let _ = adapter
                            .send_message(crate::channel::OutgoingMessage {
                                chat_id: timer.chat_id.clone(),
                                text: format!(
                                    "⏰ 定时任务 '{}' 输出:\n{}",
                                    timer.name, output
                                ),
                                parse_mode: None,
                            })
                            .await;
                    }
                }
                Err(e) => {
                    tracing::error!("定时任务 '{}' 执行失败: {}", timer.name, e);
                }
            }
        }
    });

    // 9. 启动渠道监听
    for adapter in &adapters {
        let tx = msg_tx.clone();
        let a = adapter.clone();
        tokio::spawn(async move {
            if let Err(e) = a.start(tx).await {
                tracing::error!("渠道适配器启动失败: {}", e);
            }
        });
    }
    drop(msg_tx); // 释放多余的 sender

    // 10. 消息处理主循环 - 支持优雅关闭
    let router = MessageRouter::new(executor, adapters, &config);
    
    let shutdown_signal = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("收到关闭信号 (Ctrl+C)");
    };

    let router_task = router.run(msg_rx);

    tokio::select! {
        result = router_task => {
            if let Err(e) = result {
                tracing::error!("消息路由器错误: {}", e);
            }
        }
        _ = shutdown_signal => {
            tracing::info!("正在优雅关闭...");
            // 信号已接收，select! 会继续进行
        }
    }

    // 给后台任务一些时间来清理
    tokio::time::sleep(Duration::from_secs(1)).await;

    info!("zhclaw 已停止");
    Ok(())
}
