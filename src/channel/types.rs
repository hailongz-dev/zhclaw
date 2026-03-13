use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// 渠道类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelType {
    Telegram,
    Slack,
    Discord,
    WeChat,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Telegram => write!(f, "telegram"),
            ChannelType::Slack => write!(f, "slack"),
            ChannelType::Discord => write!(f, "discord"),
            ChannelType::WeChat => write!(f, "wechat"),
        }
    }
}

/// 收到的消息（统一格式）
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// 渠道类型
    pub channel: ChannelType,
    /// 会话标识
    pub chat_id: String,
    /// 用户标识
    pub user_id: String,
    /// 消息正文
    pub text: String,
    /// 消息时间
    pub timestamp: DateTime<Utc>,
}

/// 发出的消息
#[derive(Debug, Clone)]
pub struct OutgoingMessage {
    /// 目标会话
    pub chat_id: String,
    /// 消息文本
    pub text: String,
    /// 解析模式（Markdown / HTML）
    pub parse_mode: Option<String>,
}

/// 渠道适配器 trait
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// 启动消息监听，收到消息后通过 tx 发送
    async fn start(&self, tx: mpsc::Sender<IncomingMessage>) -> anyhow::Result<()>;

    /// 发送消息到渠道
    async fn send_message(&self, msg: OutgoingMessage) -> anyhow::Result<()>;

    /// 返回渠道类型
    fn channel_type(&self) -> ChannelType;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_construction() {
        let msg = IncomingMessage {
            channel: ChannelType::Telegram,
            chat_id: "123".to_string(),
            user_id: "user1".to_string(),
            text: "hello".to_string(),
            timestamp: Utc::now(),
        };
        assert_eq!(msg.channel, ChannelType::Telegram);
        assert_eq!(msg.chat_id, "123");
        assert_eq!(msg.user_id, "user1");
        assert_eq!(msg.text, "hello");
    }

    #[test]
    fn test_outgoing_message_parse_mode_optional() {
        let msg1 = OutgoingMessage {
            chat_id: "123".to_string(),
            text: "hi".to_string(),
            parse_mode: None,
        };
        assert!(msg1.parse_mode.is_none());

        let msg2 = OutgoingMessage {
            chat_id: "123".to_string(),
            text: "hi".to_string(),
            parse_mode: Some("Markdown".to_string()),
        };
        assert_eq!(msg2.parse_mode.as_deref(), Some("Markdown"));
    }

    #[test]
    fn test_channel_type_display() {
        assert_eq!(ChannelType::Telegram.to_string(), "telegram");
        assert_eq!(ChannelType::Slack.to_string(), "slack");
        assert_eq!(ChannelType::Discord.to_string(), "discord");
        assert_eq!(ChannelType::WeChat.to_string(), "wechat");
    }

    #[test]
    fn test_channel_type_serde_roundtrip() {
        let ct = ChannelType::Telegram;
        let json = serde_json::to_string(&ct).unwrap();
        let ct2: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(ct, ct2);
    }
}
