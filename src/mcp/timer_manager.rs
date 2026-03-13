use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info};

/// 定时任务条目
#[derive(Debug, Clone, Serialize)]
pub struct TimerEntry {
    pub name: String,
    pub cron_expr: String,
    pub prompt: String,
    pub chat_id: String,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
}

/// 定时器管理器
pub struct TimerManager {
    timers: RwLock<HashMap<String, TimerEntry>>,
}

impl TimerManager {
    pub fn new() -> Self {
        Self {
            timers: RwLock::new(HashMap::new()),
        }
    }

    /// 创建定时任务
    pub async fn create_timer(
        &self,
        name: &str,
        cron_expr: &str,
        prompt: &str,
        chat_id: &str,
    ) -> Result<()> {
        // 验证 cron 表达式
        let schedule = Schedule::from_str(cron_expr)
            .map_err(|e| anyhow::anyhow!("无效的 cron 表达式 '{}': {}", cron_expr, e))?;

        let mut timers = self.timers.write().await;

        if timers.contains_key(name) {
            bail!("定时任务 '{}' 已存在", name);
        }

        let next_run = schedule.upcoming(Utc).next();

        let entry = TimerEntry {
            name: name.to_string(),
            cron_expr: cron_expr.to_string(),
            prompt: prompt.to_string(),
            chat_id: chat_id.to_string(),
            enabled: true,
            last_run: None,
            next_run,
        };

        timers.insert(name.to_string(), entry);
        info!("定时任务 '{}' 已创建 (cron: {})", name, cron_expr);
        Ok(())
    }

    /// 列出所有定时任务
    pub async fn list_timers(&self) -> Vec<TimerEntry> {
        self.timers.read().await.values().cloned().collect()
    }

    /// 删除定时任务
    pub async fn delete_timer(&self, name: &str) -> Result<()> {
        let mut timers = self.timers.write().await;
        if timers.remove(name).is_none() {
            bail!("定时任务 '{}' 不存在", name);
        }
        info!("定时任务 '{}' 已删除", name);
        Ok(())
    }

    /// 切换定时任务的启用/禁用状态
    pub async fn toggle_timer(&self, name: &str, enabled: bool) -> Result<()> {
        let mut timers = self.timers.write().await;
        match timers.get_mut(name) {
            Some(entry) => {
                entry.enabled = enabled;
                if enabled {
                    // 重新计算 next_run
                    if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                        entry.next_run = schedule.upcoming(Utc).next();
                    }
                }
                let state = if enabled { "启用" } else { "禁用" };
                info!("定时任务 '{}' 已{}", name, state);
                Ok(())
            }
            None => bail!("定时任务 '{}' 不存在", name),
        }
    }

    /// 获取所有到期的 timer（enabled 且 next_run <= now）
    pub async fn get_due_timers(&self) -> Vec<TimerEntry> {
        let now = Utc::now();
        self.timers
            .read()
            .await
            .values()
            .filter(|t| {
                t.enabled && t.next_run.map(|nr| nr <= now).unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// 标记 timer 已执行，更新 last_run 和 next_run
    pub async fn mark_executed(&self, name: &str) {
        let mut timers = self.timers.write().await;
        if let Some(entry) = timers.get_mut(name) {
            entry.last_run = Some(Utc::now());
            if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                entry.next_run = schedule.upcoming(Utc).next();
            }
        }
    }

    /// 启动调度循环
    pub fn start_scheduler<F, Fut>(
        self: &Arc<Self>,
        mut on_trigger: F,
    ) -> JoinHandle<()>
    where
        F: FnMut(TimerEntry) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                let due = manager.get_due_timers().await;
                for timer in due {
                    debug!("定时任务触发: {}", timer.name);
                    on_trigger(timer.clone()).await;
                    manager.mark_executed(&timer.name).await;
                }
            }
        })
    }
}

impl Default for TimerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_timer_success() {
        let mgr = TimerManager::new();
        let result = mgr
            .create_timer("test", "0 * * * * *", "do something", "chat1")
            .await;
        assert!(result.is_ok());

        let timers = mgr.list_timers().await;
        assert_eq!(timers.len(), 1);
        assert_eq!(timers[0].name, "test");
    }

    #[tokio::test]
    async fn test_create_timer_invalid_cron() {
        let mgr = TimerManager::new();
        let result = mgr
            .create_timer("test", "invalid_cron", "do something", "chat1")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("无效的 cron 表达式"));
    }

    #[tokio::test]
    async fn test_create_timer_duplicate_name() {
        let mgr = TimerManager::new();
        mgr.create_timer("test", "0 * * * * *", "prompt1", "chat1")
            .await
            .unwrap();
        let result = mgr
            .create_timer("test", "0 * * * * *", "prompt2", "chat1")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已存在"));
    }

    #[tokio::test]
    async fn test_delete_timer_success() {
        let mgr = TimerManager::new();
        mgr.create_timer("test", "0 * * * * *", "prompt", "chat1")
            .await
            .unwrap();
        let result = mgr.delete_timer("test").await;
        assert!(result.is_ok());
        assert!(mgr.list_timers().await.is_empty());
    }

    #[tokio::test]
    async fn test_delete_timer_not_found() {
        let mgr = TimerManager::new();
        let result = mgr.delete_timer("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不存在"));
    }

    #[tokio::test]
    async fn test_toggle_timer() {
        let mgr = TimerManager::new();
        mgr.create_timer("test", "0 * * * * *", "prompt", "chat1")
            .await
            .unwrap();

        // 默认 enabled
        let timers = mgr.list_timers().await;
        assert!(timers[0].enabled);

        // 禁用
        mgr.toggle_timer("test", false).await.unwrap();
        let timers = mgr.list_timers().await;
        assert!(!timers[0].enabled);

        // 再启用
        mgr.toggle_timer("test", true).await.unwrap();
        let timers = mgr.list_timers().await;
        assert!(timers[0].enabled);
    }

    #[tokio::test]
    async fn test_toggle_timer_not_found() {
        let mgr = TimerManager::new();
        let result = mgr.toggle_timer("nonexistent", true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_next_run_computed() {
        let mgr = TimerManager::new();
        // 每秒执行
        mgr.create_timer("test", "* * * * * *", "prompt", "chat1")
            .await
            .unwrap();
        let timers = mgr.list_timers().await;
        assert!(timers[0].next_run.is_some());
        // next_run 应该在未来
        assert!(timers[0].next_run.unwrap() >= Utc::now() - chrono::Duration::seconds(2));
    }

    #[tokio::test]
    async fn test_disabled_timer_not_due() {
        let mgr = TimerManager::new();
        // 每秒执行的 timer
        mgr.create_timer("test", "* * * * * *", "prompt", "chat1")
            .await
            .unwrap();
        // 禁用
        mgr.toggle_timer("test", false).await.unwrap();

        // 等一下让 next_run 过期
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let due = mgr.get_due_timers().await;
        assert!(due.is_empty(), "disabled timer 不应出现在 due 列表中");
    }

    #[tokio::test]
    async fn test_mark_executed_updates_times() {
        let mgr = TimerManager::new();
        mgr.create_timer("test", "* * * * * *", "prompt", "chat1")
            .await
            .unwrap();

        let before = mgr.list_timers().await;
        assert!(before[0].last_run.is_none());

        mgr.mark_executed("test").await;

        let after = mgr.list_timers().await;
        assert!(after[0].last_run.is_some());
    }

    #[tokio::test]
    async fn test_list_timers_empty() {
        let mgr = TimerManager::new();
        assert!(mgr.list_timers().await.is_empty());
    }
}
