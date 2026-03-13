use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::RwLock;
use uuid::Uuid;

/// 进程状态
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ProcessStatus {
    Running,
    Completed { exit_code: i32 },
    Failed { error: String },
    Killed,
}

/// 进程信息
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub id: String,
    pub pid: u32,
    pub command: String,
    pub chat_id: String,
    pub started_at: DateTime<Utc>,
    pub status: ProcessStatus,
}

/// 进程注册表 —— 跟踪所有运行中的 agent 子进程
pub struct ProcessRegistry {
    processes: RwLock<HashMap<String, ProcessInfo>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            processes: RwLock::new(HashMap::new()),
        }
    }

    /// 注册一个新进程，返回 process_id
    pub async fn register(&self, pid: u32, command: &str, chat_id: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let info = ProcessInfo {
            id: id.clone(),
            pid,
            command: command.to_string(),
            chat_id: chat_id.to_string(),
            started_at: Utc::now(),
            status: ProcessStatus::Running,
        };
        self.processes.write().await.insert(id.clone(), info);
        id
    }

    /// 更新进程状态
    pub async fn update_status(&self, process_id: &str, status: ProcessStatus) {
        if let Some(info) = self.processes.write().await.get_mut(process_id) {
            info.status = status;
        }
    }

    /// 列出所有 Running 状态的进程
    pub async fn list_running(&self) -> Vec<ProcessInfo> {
        self.processes
            .read()
            .await
            .values()
            .filter(|p| matches!(p.status, ProcessStatus::Running))
            .cloned()
            .collect()
    }

    /// 列出所有进程
    pub async fn list_all(&self) -> Vec<ProcessInfo> {
        self.processes.read().await.values().cloned().collect()
    }

    /// 按 ID 获取进程信息
    pub async fn get_process(&self, process_id: &str) -> Option<ProcessInfo> {
        self.processes.read().await.get(process_id).cloned()
    }

    /// 终止进程（发送 SIGTERM/kill）
    pub async fn kill_process(&self, process_id: &str) -> anyhow::Result<()> {
        let info = self
            .processes
            .read()
            .await
            .get(process_id)
            .cloned();

        match info {
            Some(info) => {
                if !matches!(info.status, ProcessStatus::Running) {
                    return Err(anyhow::anyhow!("进程 {} 不在运行状态", process_id));
                }

                // 发送 SIGTERM
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(info.pid as i32, libc::SIGTERM);
                    }
                }

                #[cfg(not(unix))]
                {
                    // Windows 等平台 fallback
                    let _ = tokio::process::Command::new("kill")
                        .arg(info.pid.to_string())
                        .output()
                        .await;
                }

                self.update_status(process_id, ProcessStatus::Killed).await;
                Ok(())
            }
            None => Err(anyhow::anyhow!("进程 {} 不存在", process_id)),
        }
    }

    /// 清理已完成的进程记录
    pub async fn cleanup_completed(&self) {
        self.processes
            .write()
            .await
            .retain(|_, p| matches!(p.status, ProcessStatus::Running));
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_returns_unique_id() {
        let registry = ProcessRegistry::new();
        let id1 = registry.register(100, "cmd1", "chat1").await;
        let id2 = registry.register(101, "cmd2", "chat2").await;
        assert_ne!(id1, id2);
    }

    #[tokio::test]
    async fn test_register_appears_in_list() {
        let registry = ProcessRegistry::new();
        let id = registry.register(100, "cmd1", "chat1").await;
        let running = registry.list_running().await;
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, id);
        assert_eq!(running[0].pid, 100);
    }

    #[tokio::test]
    async fn test_update_status_removes_from_running() {
        let registry = ProcessRegistry::new();
        let id = registry.register(100, "cmd1", "chat1").await;
        registry
            .update_status(&id, ProcessStatus::Completed { exit_code: 0 })
            .await;
        let running = registry.list_running().await;
        assert!(running.is_empty());
    }

    #[tokio::test]
    async fn test_list_running_filters_correctly() {
        let registry = ProcessRegistry::new();
        let id1 = registry.register(100, "cmd1", "chat1").await;
        let _id2 = registry.register(101, "cmd2", "chat2").await;
        registry
            .update_status(&id1, ProcessStatus::Completed { exit_code: 0 })
            .await;
        let running = registry.list_running().await;
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].pid, 101);
    }

    #[tokio::test]
    async fn test_get_process_exists() {
        let registry = ProcessRegistry::new();
        let id = registry.register(100, "cmd1", "chat1").await;
        let info = registry.get_process(&id).await;
        assert!(info.is_some());
        assert_eq!(info.unwrap().command, "cmd1");
    }

    #[tokio::test]
    async fn test_get_process_not_exists() {
        let registry = ProcessRegistry::new();
        let info = registry.get_process("nonexistent").await;
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn test_kill_process_not_exists() {
        let registry = ProcessRegistry::new();
        let result = registry.kill_process("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_register() {
        let registry = std::sync::Arc::new(ProcessRegistry::new());
        let mut handles = Vec::new();

        for i in 0..10 {
            let reg = registry.clone();
            handles.push(tokio::spawn(async move {
                reg.register(i, &format!("cmd{}", i), "chat").await
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let all = registry.list_all().await;
        assert_eq!(all.len(), 10);
    }

    #[tokio::test]
    async fn test_cleanup_completed() {
        let registry = ProcessRegistry::new();
        let id1 = registry.register(100, "cmd1", "chat1").await;
        let _id2 = registry.register(101, "cmd2", "chat2").await;
        registry
            .update_status(&id1, ProcessStatus::Completed { exit_code: 0 })
            .await;
        registry.cleanup_completed().await;
        let all = registry.list_all().await;
        assert_eq!(all.len(), 1);
    }
}
