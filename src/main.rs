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
    
    // 初始化 TimerManager 并从数据库恢复定时任务
    let timer_manager = Arc::new(
        TimerManager::new_with_db("timers.db")
            .await
            .expect("Failed to initialize timer database"),
    );
    timer_manager.load_from_db().await.ok(); // 加载已保存的定时任务

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
            info!("定时任务触发: {} (cron: {})", timer.name, timer.cron_expr);
            let start_time = chrono::Utc::now();
            
            match exec.execute(&timer.prompt, &timer.chat_id).await {
                Ok(output) => {
                    let elapsed = chrono::Utc::now().signed_duration_since(start_time);
                    let duration_str = match elapsed.num_seconds() {
                        secs if secs < 60 => format!("{}s", secs),
                        secs => format!("{}m {:.0}s", secs / 60, secs % 60),
                    };
                    
                    let notification = format!(
                        "✅ 定时任务: {}\n⏱️ 耗时: {}\n📋 输出:\n{}",
                        timer.name, duration_str, output
                    );
                    
                    info!("定时任务 '{}' 执行成功 (耗时: {})", timer.name, duration_str);
                    
                    // 通知所有渠道
                    for adapter in &adapters {
                        if let Err(e) = adapter
                            .send_message(crate::channel::OutgoingMessage {
                                chat_id: timer.chat_id.clone(),
                                text: notification.clone(),
                                parse_mode: None,
                            })
                            .await
                        {
                            tracing::warn!("发送定时任务成功通知失败: {}", e);
                        }
                    }
                }
                Err(e) => {
                    let elapsed = chrono::Utc::now().signed_duration_since(start_time);
                    let duration_str = match elapsed.num_seconds() {
                        secs if secs < 60 => format!("{}s", secs),
                        secs => format!("{}m {:.0}s", secs / 60, secs % 60),
                    };
                    
                    let error_msg = e.to_string();
                    let notification = format!(
                        "❌ 定时任务: {}\n⏱️ 耗时: {}\n⚠️ 错误: {}",
                        timer.name, duration_str, error_msg
                    );
                    
                    tracing::error!("定时任务 '{}' 执行失败 (耗时: {}): {}", timer.name, duration_str, e);
                    
                    // 通知所有渠道
                    for adapter in &adapters {
                        if let Err(notify_err) = adapter
                            .send_message(crate::channel::OutgoingMessage {
                                chat_id: timer.chat_id.clone(),
                                text: notification.clone(),
                                parse_mode: None,
                            })
                            .await
                        {
                            tracing::warn!("发送定时任务失败通知失败: {}", notify_err);
                        }
                    }
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
