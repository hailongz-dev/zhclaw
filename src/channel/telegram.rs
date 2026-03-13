use async_trait::async_trait;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::types::{ChannelAdapter, ChannelType, IncomingMessage, OutgoingMessage};

/// Telegram 消息最大长度
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// Telegram 渠道适配器
pub struct TelegramAdapter {
    bot: Bot,
}

impl TelegramAdapter {
    /// 从 TELEGRAM_BOT_TOKEN 环境变量创建
    pub fn from_env() -> Self {
        let bot = Bot::from_env();
        Self { bot }
    }

    /// 从指定 token 创建
    pub fn new(token: &str) -> Self {
        let bot = Bot::new(token);
        Self { bot }
    }

    /// 获取 bot 引用（用于测试等）
    pub fn bot(&self) -> &Bot {
        &self.bot
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

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    async fn start(&self, tx: mpsc::Sender<IncomingMessage>) -> anyhow::Result<()> {
        info!("启动 Telegram 消息监听...");

        // 重试逻辑：指数退避，最多重试 10 次
        const MAX_RETRIES: u32 = 10;
        let mut retry_count = 0;

        loop {
            // 在循环外克隆 tx，避免在循环中重复移动
            let tx_clone = tx.clone();

            let handler = Update::filter_message().endpoint(
                move |msg: Message, _bot: Bot| {
                    let tx = tx_clone.clone();
                    async move {
                        if let Some(text) = msg.text() {
                            let incoming = IncomingMessage {
                                channel: ChannelType::Telegram,
                                chat_id: msg.chat.id.to_string(),
                                user_id: msg
                                    .from
                                    .as_ref()
                                    .map(|u| u.id.to_string())
                                    .unwrap_or_default(),
                                text: text.to_string(),
                                timestamp: chrono::Utc::now(),
                            };

                            if let Err(e) = tx.send(incoming).await {
                                error!("发送消息到路由失败: {}", e);
                            }
                        }
                        respond(())
                    }
                },
            );

            match tokio::time::timeout(
                Duration::from_secs(30),
                Dispatcher::builder(self.bot.clone(), handler)
                    .enable_ctrlc_handler()
                    .build()
                    .dispatch(),
            )
            .await
            {
                Ok(_) => {
                    // 正常完成
                    break;
                }
                Err(e) => {
                    // 超时或其他错误
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        error!("Telegram 初始化失败，重试次数已达上限: {}", e);
                        return Err(anyhow::anyhow!(
                            "Telegram 初始化失败: 重试 {} 次后放弃",
                            MAX_RETRIES
                        ));
                    }

                    // 指数退避: 2^retry_count 秒，最多 120 秒
                    let delay_secs = std::cmp::min(2_u64.pow(retry_count), 120);
                    warn!(
                        "Telegram 初始化失败，{}s 后重试 (第 {}/{} 次): {}",
                        delay_secs, retry_count, MAX_RETRIES, e
                    );
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                }
            }
        }

        Ok(())
    }

    async fn send_message(&self, msg: OutgoingMessage) -> anyhow::Result<()> {
        let chat_id = ChatId(msg.chat_id.parse::<i64>().map_err(|e| {
            anyhow::anyhow!("无效的 chat_id '{}': {}", msg.chat_id, e)
        })?);

        let parts = split_message(&msg.text, TELEGRAM_MAX_MESSAGE_LENGTH);

        for part in parts {
            // 发送消息时的重试逻辑
            const MAX_RETRIES: u32 = 3;
            let mut retry_count = 0;

            loop {
                let mut request = self.bot.send_message(chat_id, &part);

                if let Some(ref mode) = msg.parse_mode {
                    match mode.to_lowercase().as_str() {
                        "markdown" | "markdownv2" => {
                            request = request.parse_mode(ParseMode::MarkdownV2);
                        }
                        "html" => {
                            request = request.parse_mode(ParseMode::Html);
                        }
                        _ => {}
                    }
                }

                match request.await {
                    Ok(_) => break, // 发送成功
                    Err(e) => {
                        retry_count += 1;
                        if retry_count >= MAX_RETRIES {
                            return Err(anyhow::anyhow!("Telegram 发送消息失败 (重试 {} 次): {}", MAX_RETRIES, e));
                        }

                        // 指数退避: 1s, 2s, 4s
                        let delay_secs = 2_u64.pow(retry_count - 1);
                        warn!(
                            "Telegram 发送消息失败，{}s 后重试 (第 {}/{} 次): {}",
                            delay_secs, retry_count, MAX_RETRIES, e
                        );
                        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                    }
                }
            }
        }

        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_type_is_telegram() {
        let adapter = TelegramAdapter::new("fake_token");
        assert_eq!(adapter.channel_type(), ChannelType::Telegram);
    }

    #[test]
    fn test_split_message_short() {
        let parts = split_message("hello", 4096);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], "hello");
    }

    #[test]
    fn test_split_message_exact() {
        let text = "a".repeat(4096);
        let parts = split_message(&text, 4096);
        assert_eq!(parts.len(), 1);
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
    fn test_split_message_prefers_newline() {
        let mut text = "a".repeat(4000);
        text.push('\n');
        text.push_str(&"b".repeat(200));
        let parts = split_message(&text, 4096);
        
        // 总长度是 4000 + 1（换行） + 200 = 4201
        // 按 4096 的限制分为：4096 + 105
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 4096);
        assert_eq!(parts[1].len(), 105);
        
        // 验证完整拼接与原始文本一致
        let joined = parts.join("");
        assert_eq!(joined, text);
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
