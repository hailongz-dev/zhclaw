use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::channel::{ChannelAdapter, ChannelType, IncomingMessage, OutgoingMessage};
use crate::config::Config;
use crate::executor::AgentExecutor;

/// 消息路由器 —— 接收来自各渠道的消息，执行 agent，发送回复
pub struct MessageRouter {
    executor: Arc<AgentExecutor>,
    adapters: HashMap<ChannelType, Arc<dyn ChannelAdapter>>,
    config: Config,
}

impl MessageRouter {
    pub fn new(
        executor: Arc<AgentExecutor>,
        adapters: Vec<Arc<dyn ChannelAdapter>>,
        config: &Config,
    ) -> Self {
        let adapter_map: HashMap<ChannelType, Arc<dyn ChannelAdapter>> = adapters
            .into_iter()
            .map(|a| (a.channel_type(), a))
            .collect();

        Self {
            executor,
            adapters: adapter_map,
            config: config.clone(),
        }
    }

    /// 主消息处理循环
    pub async fn run(&self, mut rx: mpsc::Receiver<IncomingMessage>) -> anyhow::Result<()> {
        info!("消息路由器已启动，等待消息...");

        while let Some(msg) = rx.recv().await {
            info!(
                "[{}] 收到消息: chat_id={}, user_id={}, text={}",
                msg.channel, msg.chat_id, msg.user_id,
                {
                    let chars: Vec<char> = msg.text.chars().collect();
                    if chars.len() > 50 {
                        format!("{}...", chars[..50].iter().collect::<String>())
                    } else {
                        msg.text.clone()
                    }
                }
            );

            // 权限校验
            if !self.config.is_user_allowed(&msg.user_id) {
                warn!("用户 {} 无权限，忽略消息", msg.user_id);
                if let Some(adapter) = self.adapters.get(&msg.channel) {
                    let _ = adapter
                        .send_message(OutgoingMessage {
                            chat_id: msg.chat_id.clone(),
                            text: "⛔ 你没有权限使用此机器人。".to_string(),
                            parse_mode: None,
                        })
                        .await;
                }
                continue;
            }

            // 执行 agent 命令
            let adapter = self.adapters.get(&msg.channel).cloned();
            let executor = self.executor.clone();
            let chat_id = msg.chat_id.clone();
            let text = msg.text.clone();

            // 在独立 task 中执行，避免阻塞主循环
            tokio::spawn(async move {
                // 发送 "正在处理" 提示
                if let Some(ref adapter) = adapter {
                    let _ = adapter
                        .send_message(OutgoingMessage {
                            chat_id: chat_id.clone(),
                            text: "⏳ 正在处理，请稍候...".to_string(),
                            parse_mode: None,
                        })
                        .await;
                }

                let result = executor.execute(&text, &chat_id).await;

                if let Some(adapter) = adapter {
                    let reply = match result {
                        Ok(output) => {
                            if output.trim().is_empty() {
                                "✅ 执行完成（无输出）".to_string()
                            } else {
                                output
                            }
                        }
                        Err(e) => format!("❌ 执行失败: {}", e),
                    };

                    if let Err(e) = adapter
                        .send_message(OutgoingMessage {
                            chat_id,
                            text: reply,
                            parse_mode: None,
                        })
                        .await
                    {
                        error!("发送回复失败: {}", e);
                    }
                }
            });
        }

        info!("消息路由器已停止");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::{ChannelAdapter, ChannelType, IncomingMessage, OutgoingMessage};
    use async_trait::async_trait;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Mock adapter 用于测试
    struct MockAdapter {
        channel: ChannelType,
        sent_messages: Arc<Mutex<Vec<OutgoingMessage>>>,
    }

    impl MockAdapter {
        fn new(channel: ChannelType) -> Self {
            Self {
                channel,
                sent_messages: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn sent_messages(&self) -> Vec<OutgoingMessage> {
            self.sent_messages.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelAdapter for MockAdapter {
        async fn start(&self, _tx: mpsc::Sender<IncomingMessage>) -> anyhow::Result<()> {
            Ok(())
        }

        async fn send_message(&self, msg: OutgoingMessage) -> anyhow::Result<()> {
            self.sent_messages.lock().unwrap().push(msg);
            Ok(())
        }

        fn channel_type(&self) -> ChannelType {
            self.channel
        }
    }

    fn make_test_config(allowed: &str) -> Config {
        Config {
            telegram_bot_token: "test".to_string(),
            feishu_app_id: String::new(),
            feishu_app_secret: String::new(),
            agent_command_template: "echo {prompt}".to_string(),
            agent_timeout_secs: 10,
            mcp_server_host: "0.0.0.0".to_string(),
            mcp_server_port: 3000,
            allowed_user_ids: allowed.to_string(),
            log_level: "info".to_string(),
        }
    }

    #[test]
    fn test_permission_empty_allows_all() {
        let config = make_test_config("");
        assert!(config.is_user_allowed("anyone"));
        assert!(config.is_user_allowed("user123"));
    }

    #[test]
    fn test_permission_in_list() {
        let config = make_test_config("user1,user2");
        assert!(config.is_user_allowed("user1"));
        assert!(config.is_user_allowed("user2"));
    }

    #[test]
    fn test_permission_not_in_list() {
        let config = make_test_config("user1,user2");
        assert!(!config.is_user_allowed("user3"));
    }

    #[tokio::test]
    async fn test_router_matches_adapter_by_channel() {
        let mock = Arc::new(MockAdapter::new(ChannelType::Telegram));
        let config = make_test_config("");
        let registry = Arc::new(crate::executor::process_registry::ProcessRegistry::new());
        let executor = Arc::new(AgentExecutor::new(
            "echo {prompt}",
            Duration::from_secs(10),
            registry,
        ));

        let router = MessageRouter::new(
            executor,
            vec![mock.clone() as Arc<dyn ChannelAdapter>],
            &config,
        );

        assert!(router.adapters.contains_key(&ChannelType::Telegram));
        assert!(!router.adapters.contains_key(&ChannelType::Slack));
    }
}
