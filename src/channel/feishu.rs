use async_trait::async_trait;
use chrono::{DateTime, Utc};
use feishu_sdk::api::{SendMessageBody, SendMessageQuery};
use feishu_sdk::core::{noop_logger, Config, FEISHU_BASE_URL};
use feishu_sdk::event::{Event, EventDispatcher, EventDispatcherConfig, EventHandler, EventHandlerResult};
use feishu_sdk::Client;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::types::{ChannelAdapter, ChannelType, IncomingMessage, OutgoingMessage};

/// 飞书消息最大长度
const FEISHU_MAX_MESSAGE_LENGTH: usize = 4096;

/// 飞书消息内容（text 类型）
#[derive(Debug, Serialize)]
struct FeiShuTextContent {
    text: String,
}

/// 飞书适配器 - 使用 feishu-sdk 实现
/// 
/// 架构：
/// 1. 使用 feishu-sdk 的 Client 管理认证和 API 调用
/// 2. 发送消息：调用 im.v1.message.create API
/// 3. 接收消息：通过 WebSocket 长连接（飞书推送）
/// 
/// 当前 start() 方法保持持续运行，等待 WebSocket 长连接接收消息
#[derive(Debug, Clone)]
pub struct FeiShuAdapter {
    /// feishu-sdk 的 Client，自动处理 token 获取和刷新
    client: Arc<Client>,
}

struct MessageEventHandler {
    client: Arc<Client>,
    tx: mpsc::Sender<IncomingMessage>,
}

impl EventHandler for MessageEventHandler {
    fn event_type(&self) -> &str {
        "im.message.receive_v1"
    }

    fn handle(
        &self,
        event: Event,
    ) -> Pin<Box<dyn Future<Output = EventHandlerResult> + Send + '_>> {
        let tx = self.tx.clone();
        let client = self.client.clone();

        Box::pin(async move {
            let Some(payload) = event.event else {
                warn!("飞书事件缺少 event 字段");
                return Ok(None);
            };

            let Some(message) = payload.get("message") else {
                warn!("飞书消息事件缺少 message 字段");
                return Ok(None);
            };

            let Some(chat_id) = message
                .get("chat_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
            else {
                warn!("飞书消息事件缺少 chat_id");
                return Ok(None);
            };

            let message_id = message
                .get("message_id")
                .and_then(|value| value.as_str())
                .map(str::to_string);

            let user_id = payload
                .get("sender")
                .and_then(|sender| sender.get("sender_id"))
                .and_then(|sender_id| {
                    sender_id
                        .get("open_id")
                        .or_else(|| sender_id.get("user_id"))
                        .or_else(|| sender_id.get("union_id"))
                })
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();

            let text = message
                .get("content")
                .and_then(|value| value.as_str())
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|content| {
                    content
                        .get("text")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();

            let timestamp = message
                .get("create_time")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<i64>().ok())
                .and_then(DateTime::<Utc>::from_timestamp_millis)
                .unwrap_or_else(Utc::now);

            let incoming = IncomingMessage {
                channel: ChannelType::Feishu,
                chat_id,
                user_id,
                text,
                timestamp,
            };

            if let Some(message_id) = message_id {
                let reaction = serde_json::json!({
                    "reaction_type": {
                        "emoji_type": "SMILE"
                    }
                });

                if let Err(err) = client
                    .im_v1_reaction()
                    .create()
                    .path_param("message_id", message_id)
                    .body_json(&reaction)?
                    .send()
                    .await
                {
                    warn!("飞书自动表情回复失败: {}", err);
                }
            }

            if let Err(err) = tx.send(incoming).await {
                warn!("发送飞书消息到路由失败: {}", err);
            }

            Ok(None)
        })
    }
}

impl FeiShuAdapter {
    /// 创建飞书适配器
    pub fn new(app_id: &str, app_secret: &str) -> anyhow::Result<Self> {
        let config = Config::builder(app_id, app_secret)
            .base_url(FEISHU_BASE_URL)
            .build();

        let client = Client::new(config)?;

        Ok(Self {
            client: Arc::new(client),
        })
    }
}

#[async_trait]
impl ChannelAdapter for FeiShuAdapter {
    async fn start(&self, tx: mpsc::Sender<IncomingMessage>) -> anyhow::Result<()> {
        info!("飞书机器人已启动（WebSocket 长连接模式，使用 feishu-sdk）");

        let dispatcher = EventDispatcher::new(EventDispatcherConfig::new(), noop_logger());
        dispatcher
            .register_handler(Box::new(MessageEventHandler {
                client: self.client.clone(),
                tx,
            }))
            .await;

        let stream_client = self
            .client
            .stream()
            .event_dispatcher(dispatcher)
            .build()?;

        info!("飞书机器人已进入 WebSocket 消息监听模式");
        stream_client.start().await?;
        Ok(())
    }

    async fn send_message(&self, msg: OutgoingMessage) -> anyhow::Result<()> {
        let query = SendMessageQuery {
            receive_id_type: Some("chat_id".to_string()),
        };

        // 分割长消息
        let parts = split_message(&msg.text, FEISHU_MAX_MESSAGE_LENGTH);

        for (idx, part) in parts.iter().enumerate() {
            let text_to_send = if parts.len() > 1 {
                format!("[{}/{}]\n{}", idx + 1, parts.len(), part)
            } else {
                part.clone()
            };

            // 使用 feishu-sdk 的通用 API 接口
            let content = FeiShuTextContent {
                text: text_to_send.clone(),
            };
            let body = SendMessageBody {
                receive_id: msg.chat_id.clone(),
                msg_type: "text".to_string(),
                content: serde_json::to_string(&content)?,
                uuid: None,
            };

            let resp = self
                .client
                .im_v1_message()
                .send_typed(&query, &body, Default::default())
                .await?;

            if resp.code != 0 {
                return Err(anyhow::anyhow!(
                    "飞书消息发送失败 (第 {}/{}): {}",
                    idx + 1,
                    parts.len(),
                    resp.msg,
                ));
            }
        }

        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Feishu
    }
}

/// 将长文本按指定最大长度切分为多段（字符数，不是字节数）
pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut parts = Vec::new();
    let mut start = 0;

    while start < chars.len() {
        let end = std::cmp::min(start + max_len, chars.len());
        let part: String = chars[start..end].iter().collect();
        parts.push(part);
        start = end;
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_type_is_feishu() {
        let adapter = FeiShuAdapter::new("fake_app_id", "fake_secret")
            .expect("Failed to create adapter");
        assert_eq!(adapter.channel_type(), ChannelType::Feishu);
    }

    #[test]
    fn test_split_message_short() {
        let parts = split_message("hello", 4096);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], "hello");
    }

    #[test]
    fn test_split_message_long() {
        let text = "a".repeat(5000);
        let parts = split_message(&text, 4096);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 4096);
        assert_eq!(parts[1].len(), 904);
    }

    #[test]
    fn test_split_message_multiple_parts() {
        let text = "a".repeat(10000);
        let parts = split_message(&text, 4096);
        assert_eq!(parts.len(), 3);
        let total: usize = parts.iter().map(|p| p.len()).sum();
        assert_eq!(total, 10000);
    }
}
