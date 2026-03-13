use std::time::Duration;

use serde::Deserialize;

/// 应用配置，从环境变量加载
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Telegram Bot API Token
    pub telegram_bot_token: String,

    /// 飞书应用 ID（可选）
    #[serde(default)]
    pub feishu_app_id: String,

    /// 飞书应用密钥（可选）
    #[serde(default)]
    pub feishu_app_secret: String,

    /// Agent 命令行模板，如 "claude -p {prompt}"
    pub agent_command_template: String,

    /// 命令执行超时（秒），默认 300
    #[serde(default = "default_timeout_secs")]
    pub agent_timeout_secs: u64,

    /// MCP Server 监听地址，默认 0.0.0.0
    #[serde(default = "default_mcp_host")]
    pub mcp_server_host: String,

    /// MCP Server 监听端口，默认 3000
    #[serde(default = "default_mcp_port")]
    pub mcp_server_port: u16,

    /// 允许使用的用户 ID（逗号分隔），为空则允许所有
    #[serde(default)]
    pub allowed_user_ids: String,

    /// 日志级别，默认 info
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_timeout_secs() -> u64 {
    300
}

fn default_mcp_host() -> String {
    "0.0.0.0".to_string()
}

fn default_mcp_port() -> u16 {
    3000
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    /// 从环境变量加载配置
    pub fn from_env() -> anyhow::Result<Self> {
        let config: Config = envy::from_env().map_err(|e| anyhow::anyhow!("配置加载失败: {}", e))?;
        Ok(config)
    }

    /// 获取允许的用户 ID 列表
    pub fn allowed_user_ids_list(&self) -> Vec<String> {
        if self.allowed_user_ids.trim().is_empty() {
            Vec::new()
        } else {
            self.allowed_user_ids
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
    }

    /// 获取 agent 执行超时 Duration
    pub fn agent_timeout(&self) -> Duration {
        Duration::from_secs(self.agent_timeout_secs)
    }

    /// 检查用户是否有权限
    pub fn is_user_allowed(&self, user_id: &str) -> bool {
        let allowed = self.allowed_user_ids_list();
        if allowed.is_empty() {
            return true; // 空列表 = 允许所有
        }
        allowed.iter().any(|id| id == user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助函数：在临时环境中构造 Config
    fn make_config(
        token: &str,
        template: &str,
        timeout: Option<u64>,
        allowed: &str,
    ) -> Config {
        Config {
            telegram_bot_token: token.to_string(),
            feishu_app_id: String::new(),
            feishu_app_secret: String::new(),
            agent_command_template: template.to_string(),
            agent_timeout_secs: timeout.unwrap_or(default_timeout_secs()),
            mcp_server_host: default_mcp_host(),
            mcp_server_port: default_mcp_port(),
            allowed_user_ids: allowed.to_string(),
            log_level: default_log_level(),
        }
    }

    #[test]
    fn test_config_fields_correct() {
        let config = make_config("token123", "claude -p {prompt}", Some(600), "");
        assert_eq!(config.telegram_bot_token, "token123");
        assert_eq!(config.agent_command_template, "claude -p {prompt}");
        assert_eq!(config.agent_timeout_secs, 600);
    }

    #[test]
    fn test_default_values() {
        let config = make_config("t", "cmd", None, "");
        assert_eq!(config.agent_timeout_secs, 300);
        assert_eq!(config.mcp_server_host, "0.0.0.0");
        assert_eq!(config.mcp_server_port, 3000);
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_allowed_user_ids_empty() {
        let config = make_config("t", "cmd", None, "");
        assert!(config.allowed_user_ids_list().is_empty());
    }

    #[test]
    fn test_allowed_user_ids_whitespace_only() {
        let config = make_config("t", "cmd", None, "   ");
        assert!(config.allowed_user_ids_list().is_empty());
    }

    #[test]
    fn test_allowed_user_ids_parse() {
        let config = make_config("t", "cmd", None, "a, b ,c");
        let ids = config.allowed_user_ids_list();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_is_user_allowed_empty_allows_all() {
        let config = make_config("t", "cmd", None, "");
        assert!(config.is_user_allowed("anyone"));
    }

    #[test]
    fn test_is_user_allowed_in_list() {
        let config = make_config("t", "cmd", None, "user1,user2");
        assert!(config.is_user_allowed("user1"));
        assert!(config.is_user_allowed("user2"));
    }

    #[test]
    fn test_is_user_allowed_not_in_list() {
        let config = make_config("t", "cmd", None, "user1,user2");
        assert!(!config.is_user_allowed("user3"));
    }

    #[test]
    fn test_agent_timeout_duration() {
        let config = make_config("t", "cmd", Some(120), "");
        assert_eq!(config.agent_timeout(), Duration::from_secs(120));
    }
}
