use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use rusqlite::{params, Connection};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
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

/// 定时器管理器（支持 SQLite 持久化）
pub struct TimerManager {
    timers: RwLock<HashMap<String, TimerEntry>>,
    db: Mutex<Connection>,
}

impl TimerManager {
    /// 创建或加载定时器管理器
    pub async fn new_with_db(db_path: &str) -> Result<Self> {
        // 在同步线程中初始化数据库
        let db_path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&db_path)?;

            // 创建 timers 表
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS timers (
                    name TEXT PRIMARY KEY,
                    cron_expr TEXT NOT NULL,
                    prompt TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    enabled INTEGER NOT NULL,
                    last_run TEXT,
                    next_run TEXT,
                    created_at TEXT NOT NULL
                );
                "#,
            )?;

            Ok::<_, anyhow::Error>(conn)
        })
        .await??;

        Ok(Self {
            timers: RwLock::new(HashMap::new()),
            db: Mutex::new(conn),
        })
    }

    /// 仅内存模式（用于测试）
    pub fn new() -> Self {
        // 创建内存数据库
        let conn = Connection::open_in_memory().expect("Failed to create in-memory DB");
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS timers (
                name TEXT PRIMARY KEY,
                cron_expr TEXT NOT NULL,
                prompt TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                last_run TEXT,
                next_run TEXT,
                created_at TEXT NOT NULL
            );
            "#,
        )
        .expect("Failed to create timers table");

        Self {
            timers: RwLock::new(HashMap::new()),
            db: Mutex::new(conn),
        }
    }

    /// 从数据库恢复所有定时任务
    pub async fn load_from_db(&self) -> Result<()> {
        let db = self.db.lock().await;
        let db_path = db.path().ok_or_else(|| anyhow::anyhow!("Database path not available"))?;
        let db_path = db_path.to_string();

        let entries = tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&db_path)?;
            let mut stmt = conn.prepare("SELECT name, cron_expr, prompt, chat_id, enabled, last_run, next_run FROM timers")?;

            let timers = stmt.query_map([], |row| {
                Ok(TimerEntry {
                    name: row.get(0)?,
                    cron_expr: row.get(1)?,
                    prompt: row.get(2)?,
                    chat_id: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    last_run: row.get::<_, Option<String>>(5)?.and_then(|s| {
                        DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))
                    }),
                    next_run: row.get::<_, Option<String>>(6)?.and_then(|s| {
                        DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))
                    }),
                })
            })?;

            let mut all_timers = HashMap::new();
            for timer_result in timers {
                let timer = timer_result?;
                all_timers.insert(timer.name.clone(), timer);
            }
            Ok::<_, anyhow::Error>(all_timers)
        })
        .await??;

        let mut timers = self.timers.write().await;
        *timers = entries;

        info!("从数据库恢复 {} 个定时任务", timers.len());
        Ok(())
    }

    /// 创建定时任务并写入数据库
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

        // 写入数据库
        let db = self.db.lock().await;
        let db_path = db.path()
            .ok_or_else(|| anyhow::anyhow!("Database path not available"))?
            .to_string();
        drop(db); // 释放锁
        
        let entry_clone = entry.clone();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path)?;
            conn.execute(
                "INSERT INTO timers (name, cron_expr, prompt, chat_id, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    &entry_clone.name,
                    &entry_clone.cron_expr,
                    &entry_clone.prompt,
                    &entry_clone.chat_id,
                    1i32,
                    Utc::now().to_rfc3339(),
                ],
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .await??;

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

        // 从数据库删除
        let db = self.db.lock().await;
        let db_path = db.path()
            .ok_or_else(|| anyhow::anyhow!("Database path not available"))?
            .to_string();
        drop(db);
        
        let name_to_delete = name.to_string();
        let name_for_log = name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path)?;
            conn.execute("DELETE FROM timers WHERE name = ?1", params![&name_to_delete])?;
            Ok::<_, anyhow::Error>(())
        })
        .await??;

        info!("定时任务 '{}' 已删除", name_for_log);
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

                // 更新数据库
                let db = self.db.lock().await;
                let db_path = db.path()
                    .ok_or_else(|| anyhow::anyhow!("Database path not available"))?
                    .to_string();
                drop(db);
                
                let entry_clone = entry.clone();
                tokio::task::spawn_blocking(move || {
                    let conn = Connection::open(db_path)?;
                    let next_run_str = entry_clone.next_run.map(|dt| dt.to_rfc3339());
                    conn.execute(
                        "UPDATE timers SET enabled = ?1, next_run = ?2 WHERE name = ?3",
                        params![
                            if enabled { 1i32 } else { 0i32 },
                            next_run_str,
                            &entry_clone.name,
                        ],
                    )?;
                    Ok::<_, anyhow::Error>(())
                })
                .await??;

                let state = if enabled { "启用" } else { "禁用" };
                info!("定时任务 '{}' 已{}", name, state);
                Ok(())
            }
            None => bail!("定时任务 '{}' 不存在", name),
        }
    }

    /// 更新定时任务的 next_run 和 last_run
    pub async fn update_timer_run(&self, name: &str, next_run: Option<DateTime<Utc>>) -> Result<()> {
        let mut timers = self.timers.write().await;
        if let Some(entry) = timers.get_mut(name) {
            entry.last_run = Some(Utc::now());
            entry.next_run = next_run;

            // 更新数据库
            let db = self.db.lock().await;
            let db_path = db.path()
                .ok_or_else(|| anyhow::anyhow!("Database path not available"))?
                .to_string();
            drop(db);
            
            let entry_clone = entry.clone();
            tokio::task::spawn_blocking(move || {
                let conn = Connection::open(db_path)?;
                let last_run_str = entry_clone.last_run.map(|dt| dt.to_rfc3339());
                let next_run_str = entry_clone.next_run.map(|dt| dt.to_rfc3339());
                conn.execute(
                    "UPDATE timers SET last_run = ?1, next_run = ?2 WHERE name = ?3",
                    params![last_run_str, next_run_str, &entry_clone.name],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .await??;

            debug!("定时任务 '{}' 执行完成，下次运行: {:?}", name, next_run);
            Ok(())
        } else {
            bail!("定时任务 '{}' 不存在", name)
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
        let timers = self.timers.write().await;
        let next_run = if let Some(entry) = timers.get(name) {
            if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                schedule.upcoming(Utc).next()
            } else {
                None
            }
        } else {
            return;
        };
        drop(timers); // 释放锁

        let _ = self.update_timer_run(name, next_run).await;
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

    async fn create_test_manager() -> TimerManager {
        let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let db_path = temp_file.path().to_string_lossy().to_string();
        // 关闭文件以释放锁
        drop(temp_file);
        TimerManager::new_with_db(&db_path).await.expect("Failed to create test manager")
    }

    #[tokio::test]
    async fn test_create_timer_success() {
        let mgr = create_test_manager().await;
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
        let mgr = create_test_manager().await;
        let result = mgr
            .create_timer("test", "invalid_cron", "do something", "chat1")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("无效的 cron 表达式"));
    }

    #[tokio::test]
    async fn test_create_timer_duplicate_name() {
        let mgr = create_test_manager().await;
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
        let mgr = create_test_manager().await;
        mgr.create_timer("test", "0 * * * * *", "prompt", "chat1")
            .await
            .unwrap();
        let result = mgr.delete_timer("test").await;
        assert!(result.is_ok());
        assert!(mgr.list_timers().await.is_empty());
    }

    #[tokio::test]
    async fn test_delete_timer_not_found() {
        let mgr = create_test_manager().await;
        let result = mgr.delete_timer("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不存在"));
    }

    #[tokio::test]
    async fn test_toggle_timer() {
        let mgr = create_test_manager().await;
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
        let mgr = create_test_manager().await;
        let result = mgr.toggle_timer("nonexistent", true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_next_run_computed() {
        let mgr = create_test_manager().await;
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
        let mgr = create_test_manager().await;
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
        let mgr = create_test_manager().await;
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
        let mgr = create_test_manager().await;
        assert!(mgr.list_timers().await.is_empty());
    }
}
