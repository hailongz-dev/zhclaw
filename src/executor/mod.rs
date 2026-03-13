pub mod process_registry;

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use shell_escape::escape;
use tracing::{debug, error, info};

use crate::executor::process_registry::{ProcessRegistry, ProcessStatus};

/// Agent 命令行执行引擎
pub struct AgentExecutor {
    /// 命令模板，如 "claude -p {prompt}"
    command_template: String,
    /// 执行超时
    timeout: Duration,
    /// 进程注册表
    process_registry: Arc<ProcessRegistry>,
}

impl AgentExecutor {
    pub fn new(
        command_template: &str,
        timeout: Duration,
        process_registry: Arc<ProcessRegistry>,
    ) -> Self {
        Self {
            command_template: command_template.to_string(),
            timeout,
            process_registry,
        }
    }

    /// 渲染命令模板，替换 {prompt} 并做 shell escape
    pub fn render_command(&self, prompt: &str) -> String {
        let escaped = escape(std::borrow::Cow::Borrowed(prompt));
        self.command_template.replace("{prompt}", &escaped)
    }

    /// 为 prompt 注入渠道上下文，便于 agent 感知消息来源
    pub fn prompt_with_context(prompt: &str, channel: &str, chat_id: &str) -> String {
        format!("channel={}, chat_id={}\n{}", channel, chat_id, prompt)
    }

    /// 解析命令字符串为 (程序, 参数列表)，正确处理转义和引号
    pub fn parse_command(command: &str) -> (String, Vec<String>) {
        match shlex::split(command) {
            Some(parts) if !parts.is_empty() => {
                let cmd = parts[0].clone();
                let args = parts[1..].to_vec();
                (cmd, args)
            }
            _ => (String::new(), Vec::new()),
        }
    }

    /// 执行 agent 命令，返回输出文本
    pub async fn execute(&self, prompt: &str, chat_id: &str) -> Result<String> {
        let command = self.render_command(prompt);
        info!("执行命令: {}", command);

        let (cmd, args) = Self::parse_command(&command);

        let child = tokio::process::Command::new(&cmd)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("启动命令 '{}' 失败", cmd))?;

        // 获取 pid 并注册
        let pid = child.id().unwrap_or(0);
        let process_id = self
            .process_registry
            .register(pid, &command, chat_id)
            .await;

        debug!("进程已注册: id={}, pid={}", process_id, pid);

        // 带超时的执行
        let result = tokio::time::timeout(self.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                self.process_registry
                    .update_status(&process_id, ProcessStatus::Completed { exit_code })
                    .await;

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut result_text = stdout.to_string();
                if !stderr.is_empty() {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str("[stderr] ");
                    result_text.push_str(&stderr);
                }

                info!("命令完成: exit_code={}", exit_code);
                Ok(result_text)
            }
            Ok(Err(e)) => {
                self.process_registry
                    .update_status(
                        &process_id,
                        ProcessStatus::Failed {
                            error: e.to_string(),
                        },
                    )
                    .await;
                Err(anyhow::anyhow!("命令执行失败: {}", e))
            }
            Err(_) => {
                // 超时 — 尝试 kill 子进程
                error!("命令执行超时 ({}s)", self.timeout.as_secs());
                self.process_registry
                    .update_status(&process_id, ProcessStatus::Killed)
                    .await;
                // 尝试 kill
                let _ = self.process_registry.kill_process(&process_id).await;
                Err(anyhow::anyhow!(
                    "命令执行超时 ({}s)",
                    self.timeout.as_secs()
                ))
            }
        }
    }

    /// 执行带渠道上下文的 agent 命令，返回输出文本
    pub async fn execute_with_context(&self, prompt: &str, channel: &str, chat_id: &str) -> Result<String> {
        let prompt = Self::prompt_with_context(prompt, channel, chat_id);
        self.execute(&prompt, chat_id).await
    }

    /// 获取进程注册表引用
    pub fn process_registry(&self) -> &Arc<ProcessRegistry> {
        &self.process_registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor(template: &str, timeout_secs: u64) -> AgentExecutor {
        let registry = Arc::new(ProcessRegistry::new());
        AgentExecutor::new(template, Duration::from_secs(timeout_secs), registry)
    }

    #[test]
    fn test_render_command_simple() {
        let exec = make_executor("echo {prompt}", 30);
        let cmd = exec.render_command("hello world");
        assert_eq!(cmd, "echo 'hello world'");
    }

    #[test]
    fn test_prompt_with_context() {
        let prompt = AgentExecutor::prompt_with_context("hello", "telegram", "123");
        assert_eq!(prompt, "channel=telegram, chat_id=123\nhello");
    }

    #[test]
    fn test_render_command_special_chars() {
        let exec = make_executor("echo {prompt}", 30);
        let cmd = exec.render_command("it's a \"test\" $var");
        // shell_escape 应该对特殊字符做转义
        assert!(cmd.starts_with("echo "));
        assert!(cmd.contains("it"));
        assert!(cmd.contains("test"));
    }

    #[test]
    fn test_parse_command_basic() {
        let (cmd, args) = AgentExecutor::parse_command("echo hello world");
        assert_eq!(cmd, "echo");
        assert_eq!(args, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_command_single() {
        let (cmd, args) = AgentExecutor::parse_command("ls");
        assert_eq!(cmd, "ls");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_command_empty() {
        let (cmd, args) = AgentExecutor::parse_command("");
        assert!(cmd.is_empty());
        assert!(args.is_empty());
    }

    #[tokio::test]
    async fn test_execute_echo() {
        let exec = make_executor("echo {prompt}", 30);
        let result = exec.execute("hello", "chat1").await.unwrap();
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_nonexistent_command() {
        let exec = make_executor("nonexistent_command_xyz {prompt}", 5);
        let result = exec.execute("test", "chat1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let exec = make_executor("sleep {prompt}", 1);
        let result = exec.execute("10", "chat1").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("超时"));
    }

    #[tokio::test]
    async fn test_execute_captures_stderr() {
        let exec = make_executor("sh -c {prompt}", 5);
        let result = exec.execute("echo err >&2", "chat1").await.unwrap();
        assert!(result.contains("stderr"));
        assert!(result.contains("err"));
    }

    #[tokio::test]
    async fn test_execute_nonzero_exit() {
        let exec = make_executor("sh -c {prompt}", 5);
        let result = exec.execute("exit 1", "chat1").await;
        // 非零退出码不是 error，输出仍应被捕获
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_registered_during_execution() {
        let registry = Arc::new(ProcessRegistry::new());
        let exec = AgentExecutor::new("sleep 2", Duration::from_secs(10), registry.clone());

        // 在后台启动执行
        let handle = tokio::spawn(async move {
            exec.execute("", "chat1").await
        });

        // 短暂等待让进程启动
        tokio::time::sleep(Duration::from_millis(200)).await;

        let running = registry.list_running().await;
        assert!(!running.is_empty(), "执行中应有进程在注册表中");

        let _ = handle.await;

        // 执行完成后
        let running = registry.list_running().await;
        assert!(running.is_empty(), "执行完成后注册表应无 running 进程");
    }
}
