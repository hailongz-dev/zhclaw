use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::types::{ChannelAdapter, ChannelType, IncomingMessage, OutgoingMessage};

/// 飞书消息最大长度
const FEISHU_MAX_MESSAGE_LENGTH: usize = 4096;

/// 飞书适配器配置
#[derive(Debug, Clone)]
pub struct FeiShuAdapter {
    /// 飞书应用 ID
    app_id: String,
    /// 飞书应用密钥
    app_secret: String,
    /// HTTP 客户端（可选，用于发送消息）
    http_client: Arc<reqwest::Client>,
}

/// 飞书 API 响应结构
#[derive(Debug, Deserialize)]
struct FeiShuApiResponse {
    code: i32,
    msg: String,
    #[serde(default)]
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

/// 飞书获取 tenant_access_token 的响应
#[derive(Debug, Deserialize)]
struct FeiShuTokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: Option<String>,
    #[allow(dead_code)]
    expire: Option<i32>,
}

/// 飞书消息结构
#[derive(Debug, Serialize)]
struct FeiShuMessage {
    receive_id: String,
    msg_type: String,
    content: String,
}

/// 飞书消息内容（text 类型）
#[derive(Debug, Serialize)]
struct FeiShuTextContent {
    text: String,
}

impl FeiShuAdapter {
    /// 创建飞书适配器
    pub fn new(app_id: &str, app_secret: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            http_client: Arc::new(reqwest::Client::new()),
        }
    }

    /// 获取 tenant_access_token
    async fn get_access_token(&self) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "app_id": &self.app_id,
            "app_secret": &self.app_secret,
        });

        let response = self
            .http_client
            .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
            .json(&body)
            .send()
            .await?;

        let token_response: FeiShuTokenResponse = response.json().await?;

        if token_response.code != 0 {
            return Err(anyhow::anyhow!(
                "飞书 token 获取失败: {}",
                token_response.msg
            ));
        }

        token_response
            .tenant_access_token
            .ok_or_else(|| anyhow::anyhow!("飞书返回的 token 为空"))
    }

    /// 发送单条消息
    async fn send_single_message(
        &self,
        chat_id: &str,
        text: &str,
        access_token: &str,
    ) -> anyhow::Result<()> {
        let content = FeiShuTextContent {
            text: text.to_string(),
        };

        let message = FeiShuMessage {
            receive_id: chat_id.to_string(),
            msg_type: "text".to_string(),
            content: serde_json::to_string(&content)?,
        };

        let response = self
            .http_client
            .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&message)
            .send()
            .await?;

        let api_response: FeiShuApiResponse = response.json().await?;

        if api_response.code != 0 {
            return Err(anyhow::anyhow!(
                "飞书消息发送失败: {}",
                api_response.msg
            ));
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for FeiShuAdapter {
    async fn start(&self, _tx: mpsc::Sender<IncomingMessage>) -> anyhow::Result<()> {
        info!("飞书机器人处于待机状态（通过 Webhook 接收消息）");
        
        // 飞书通过 webhook 回调，不需要主动轮询
        // 这里仅作为占位符，实际消息接收需要在 HTTP 服务器中处理
        // 应用应该在 main.rs 中实现一个 HTTP 路由来接收飞书的 webhook 回调
        
        // 为了让应用能正常启动，这里返回 Ok 并让消息通过主程序的 HTTP 服务器接收
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }

    async fn send_message(&self, msg: OutgoingMessage) -> anyhow::Result<()> {
        // 发送消息时的重试逻辑
        const MAX_RETRIES: u32 = 3;
        let mut retry_count = 0;

        loop {
            // 获取 access token
            let token = match self.get_access_token().await {
                Ok(t) => t,
                Err(e) => {
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(anyhow::anyhow!("飞书 token 获取失败 (重试 {} 次): {}", MAX_RETRIES, e));
                    }
                    let delay_secs = 2_u64.pow(retry_count - 1);
                    warn!(
                        "飞书 token 获取失败，{}s 后重试 (第 {}/{} 次): {}",
                        delay_secs, retry_count, MAX_RETRIES, e
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
                    continue;
                }
            };

            // 分割长消息
            let parts = split_message(&msg.text, FEISHU_MAX_MESSAGE_LENGTH);

            for (idx, part) in parts.iter().enumerate() {
                let text_to_send = if parts.len() > 1 {
                    format!("[{}/{}]\n{}", idx + 1, parts.len(), part)
                } else {
                    part.clone()
                };

                match self
                    .send_single_message(&msg.chat_id, &text_to_send, &token)
                    .await
                {
                    Ok(_) => {
                        // 消息发送成功
                    }
                    Err(e) => {
                        retry_count += 1;
                        if retry_count >= MAX_RETRIES {
                            return Err(anyhow::anyhow!(
                                "飞书消息发送失败 (重试 {} 次): {}",
                                MAX_RETRIES, e
                            ));
                        }

                        let delay_secs = 2_u64.pow(retry_count - 1);
                        warn!(
                            "飞书消息发送失败，{}s 后重试 (第 {}/{} 次): {}",
                            delay_secs, retry_count, MAX_RETRIES, e
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
                        continue;
                    }
                }
            }

            // 所有部分都发送成功
            break;
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
        let adapter = FeiShuAdapter::new("fake_app_id", "fake_secret");
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
