use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Local, Utc};
use cron::Schedule;
use rusqlite::{params, Connection};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// 定时任务条目
#[derive(Debug, Clone, Serialize)]
pub struct TimerEntry {
    pub name: String,
    pub cron_expr: String,
    pub prompt: String,
    pub channel: Option<String>,
    pub chat_id: String,
    pub max_trigger_count: i64,
    pub trigger_count: i64,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
}

/// 定时器管理器（支持 SQLite 持久化）
pub struct TimerManager {
    timers: RwLock<HashMap<String, TimerEntry>>,
    db: Mutex<Connection>,
}

fn compute_next_run(schedule: &Schedule) -> Option<DateTime<Utc>> {
    schedule
        .upcoming(Local)
        .next()
        .map(|next| next.with_timezone(&Utc))
}

fn infer_channel_from_chat_id(chat_id: &str) -> Option<String> {
    if chat_id.parse::<i64>().is_ok() {
        Some("telegram".to_string())
    } else if chat_id.starts_with("oc_") {
        Some("feishu".to_string())
    } else {
        None
    }
}

fn normalize_channel(channel: &str) -> Option<String> {
    match channel.trim().to_lowercase().as_str() {
        "telegram" => Some("telegram".to_string()),
        "feishu" => Some("feishu".to_string()),
        "slack" => Some("slack".to_string()),
        "discord" => Some("discord".to_string()),
        "wechat" => Some("wechat".to_string()),
        _ => None,
    }
}

fn validate_chat_id_for_channel(channel: &str, chat_id: &str) -> Result<()> {
    match channel {
        "telegram" if chat_id.parse::<i64>().is_err() => {
            bail!("Telegram 定时任务的 chat_id 必须是整数，当前值: '{}'", chat_id)
        }
        "feishu" if !chat_id.starts_with("oc_") => {
            bail!("Feishu 定时任务的 chat_id 必须以 'oc_' 开头，当前值: '{}'", chat_id)
        }
        _ => Ok(()),
    }
}

fn resolve_channel(channel: Option<&str>, chat_id: &str) -> Result<String> {
    match channel {
        Some(channel) => {
            let normalized = normalize_channel(channel)
                .ok_or_else(|| anyhow::anyhow!("不支持的 channel: '{}'", channel))?;
            validate_chat_id_for_channel(&normalized, chat_id)?;
            Ok(normalized)
        }
        None => infer_channel_from_chat_id(chat_id).ok_or_else(|| {
            anyhow::anyhow!(
                "无法从 chat_id '{}' 推断 channel，请显式传入 channel（如 telegram / feishu）",
                chat_id
            )
        }),
    }
}

fn validate_max_trigger_count(max_trigger_count: i64) -> Result<()> {
    if max_trigger_count == -1 || max_trigger_count > 0 {
        Ok(())
    } else {
        bail!("max_trigger_count 必须为 -1（不限制）或正整数，当前值: {}", max_trigger_count)
    }
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
                    channel TEXT,
                    chat_id TEXT NOT NULL,
                    max_trigger_count INTEGER NOT NULL DEFAULT -1,
                    trigger_count INTEGER NOT NULL DEFAULT 0,
                    enabled INTEGER NOT NULL,
                    last_run TEXT,
                    next_run TEXT,
                    created_at TEXT NOT NULL
                );
                "#,
            )?;

            let _ = conn.execute("ALTER TABLE timers ADD COLUMN channel TEXT", []);
            let _ = conn.execute(
                "ALTER TABLE timers ADD COLUMN max_trigger_count INTEGER NOT NULL DEFAULT -1",
                [],
            );
            let _ = conn.execute(
                "ALTER TABLE timers ADD COLUMN trigger_count INTEGER NOT NULL DEFAULT 0",
                [],
            );

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
                channel TEXT,
                chat_id TEXT NOT NULL,
                max_trigger_count INTEGER NOT NULL DEFAULT -1,
                trigger_count INTEGER NOT NULL DEFAULT 0,
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
            let mut stmt = conn.prepare("SELECT name, cron_expr, prompt, channel, chat_id, max_trigger_count, trigger_count, enabled, last_run, next_run FROM timers")?;

            let timers = stmt.query_map([], |row| {
                Ok(TimerEntry {
                    name: row.get(0)?,
                    cron_expr: row.get(1)?,
                    prompt: row.get(2)?,
                    channel: row.get::<_, Option<String>>(3)?,
                    chat_id: row.get(4)?,
                    max_trigger_count: row.get(5)?,
                    trigger_count: row.get(6)?,
                    enabled: row.get::<_, i32>(7)? != 0,
                    last_run: row.get::<_, Option<String>>(8)?.and_then(|s| {
                        DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))
                    }),
                    next_run: row.get::<_, Option<String>>(9)?.and_then(|s| {
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

        let mut normalized_entries = entries;
        for timer in normalized_entries.values_mut() {
            if timer.channel.is_none() {
                match infer_channel_from_chat_id(&timer.chat_id) {
                    Some(channel) => timer.channel = Some(channel),
                    None => {
                        timer.enabled = false;
                        timer.next_run = None;
                        warn!(
                            "定时任务 '{}' 的 chat_id='{}' 无法推断渠道，已自动禁用；请补充 channel 或使用合法 chat_id 重建",
                            timer.name,
                            timer.chat_id
                        );
                        continue;
                    }
                }
            }

            if let Some(channel) = timer.channel.as_deref() {
                if let Err(err) = validate_chat_id_for_channel(channel, &timer.chat_id) {
                    timer.enabled = false;
                    timer.next_run = None;
                    warn!(
                        "定时任务 '{}' 配置无效，已自动禁用: {}",
                        timer.name,
                        err
                    );
                    continue;
                }
            }

            if timer.max_trigger_count != -1 && timer.trigger_count >= timer.max_trigger_count {
                timer.enabled = false;
                timer.next_run = None;
            } else if timer.enabled {
                if let Ok(schedule) = Schedule::from_str(&timer.cron_expr) {
                    timer.next_run = compute_next_run(&schedule);
                }
            }
        }

        let mut timers = self.timers.write().await;
        *timers = normalized_entries;

        info!("从数据库恢复 {} 个定时任务", timers.len());
        Ok(())
    }

    /// 创建定时任务并写入数据库
    pub async fn create_timer(
        &self,
        name: &str,
        cron_expr: &str,
        prompt: &str,
        channel: Option<&str>,
        chat_id: &str,
    ) -> Result<()> {
        self.create_timer_with_limit(name, cron_expr, prompt, channel, chat_id, -1)
            .await
    }

    /// 创建定时任务并写入数据库，可指定最大触发次数（-1 表示不限制）
    pub async fn create_timer_with_limit(
        &self,
        name: &str,
        cron_expr: &str,
        prompt: &str,
        channel: Option<&str>,
        chat_id: &str,
        max_trigger_count: i64,
    ) -> Result<()> {
        // 验证 cron 表达式
        let schedule = Schedule::from_str(cron_expr)
            .map_err(|e| anyhow::anyhow!("无效的 cron 表达式 '{}': {}", cron_expr, e))?;
        validate_max_trigger_count(max_trigger_count)?;

        let channel = resolve_channel(channel, chat_id)?;

        let mut timers = self.timers.write().await;

        if timers.contains_key(name) {
            bail!("定时任务 '{}' 已存在", name);
        }

        let next_run = compute_next_run(&schedule);

        let entry = TimerEntry {
            name: name.to_string(),
            cron_expr: cron_expr.to_string(),
            prompt: prompt.to_string(),
            channel: Some(channel.clone()),
            chat_id: chat_id.to_string(),
            max_trigger_count,
            trigger_count: 0,
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
                "INSERT INTO timers (name, cron_expr, prompt, channel, chat_id, max_trigger_count, trigger_count, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    &entry_clone.name,
                    &entry_clone.cron_expr,
                    &entry_clone.prompt,
                    &entry_clone.channel,
                    &entry_clone.chat_id,
                    entry_clone.max_trigger_count,
                    entry_clone.trigger_count,
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
                if enabled && entry.max_trigger_count != -1 && entry.trigger_count >= entry.max_trigger_count {
                    bail!("定时任务 '{}' 已达到最大触发次数 {}，无法重新启用", name, entry.max_trigger_count);
                }

                entry.enabled = enabled;
                if enabled {
                    // 重新计算 next_run
                    if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                        entry.next_run = compute_next_run(&schedule);
                    }
                } else {
                    entry.next_run = None;
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
    pub async fn update_timer_run(
        &self,
        name: &str,
        next_run: Option<DateTime<Utc>>,
        enabled: bool,
        trigger_count: i64,
    ) -> Result<()> {
        let mut timers = self.timers.write().await;
        if let Some(entry) = timers.get_mut(name) {
            entry.last_run = Some(Local::now().with_timezone(&Utc));
            entry.next_run = next_run;
            entry.enabled = enabled;
            entry.trigger_count = trigger_count;

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
                    "UPDATE timers SET last_run = ?1, next_run = ?2, enabled = ?3, trigger_count = ?4 WHERE name = ?5",
                    params![
                        last_run_str,
                        next_run_str,
                        if entry_clone.enabled { 1i32 } else { 0i32 },
                        entry_clone.trigger_count,
                        &entry_clone.name
                    ],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .await??;

            debug!(
                "定时任务 '{}' 执行完成，触发次数: {}，下次运行: {:?}",
                name,
                trigger_count,
                next_run
            );
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
        let (next_run, enabled, trigger_count, max_trigger_count) = if let Some(entry) = timers.get(name) {
            let trigger_count = entry.trigger_count + 1;
            let reached_limit = entry.max_trigger_count != -1 && trigger_count >= entry.max_trigger_count;
            if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                (
                    if reached_limit {
                        None
                    } else {
                        compute_next_run(&schedule)
                    },
                    !reached_limit,
                    trigger_count,
                    entry.max_trigger_count,
                )
            } else {
                (None, false, trigger_count, entry.max_trigger_count)
            }
        } else {
            return;
        };
        drop(timers); // 释放锁

        let _ = self.update_timer_run(name, next_run, enabled, trigger_count).await;

        if max_trigger_count != -1 && trigger_count >= max_trigger_count {
            info!(
                "定时任务 '{}' 已达到最大触发次数 {}，自动停止",
                name,
                max_trigger_count
            );
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
            .create_timer("test", "0 * * * * *", "do something", Some("telegram"), "123")
            .await;
        assert!(result.is_ok());

        let timers = mgr.list_timers().await;
        assert_eq!(timers.len(), 1);
        assert_eq!(timers[0].name, "test");
        assert_eq!(timers[0].max_trigger_count, -1);
        assert_eq!(timers[0].trigger_count, 0);
    }

    #[tokio::test]
    async fn test_create_timer_invalid_cron() {
        let mgr = create_test_manager().await;
        let result = mgr
            .create_timer("test", "invalid_cron", "do something", Some("telegram"), "123")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("无效的 cron 表达式"));
    }

    #[tokio::test]
    async fn test_create_timer_duplicate_name() {
        let mgr = create_test_manager().await;
        mgr.create_timer("test", "0 * * * * *", "prompt1", Some("telegram"), "123")
            .await
            .unwrap();
        let result = mgr
            .create_timer("test", "0 * * * * *", "prompt2", Some("telegram"), "123")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已存在"));
    }

    #[tokio::test]
    async fn test_delete_timer_success() {
        let mgr = create_test_manager().await;
        mgr.create_timer("test", "0 * * * * *", "prompt", Some("telegram"), "123")
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
        mgr.create_timer("test", "0 * * * * *", "prompt", Some("telegram"), "123")
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
        mgr.create_timer("test", "* * * * * *", "prompt", Some("telegram"), "123")
            .await
            .unwrap();
        let timers = mgr.list_timers().await;
        assert!(timers[0].next_run.is_some());
        // next_run 应该在未来
        assert!(timers[0].next_run.unwrap() >= Utc::now() - chrono::Duration::seconds(2));
    }

    #[tokio::test]
    async fn test_next_run_uses_local_timezone() {
        let mgr = create_test_manager().await;
        let cron_expr = "0 0 9 * * *";

        mgr.create_timer("local-time", cron_expr, "prompt", Some("telegram"), "123")
            .await
            .unwrap();

        let timers = mgr.list_timers().await;
        let expected = Schedule::from_str(cron_expr)
            .unwrap()
            .upcoming(Local)
            .next()
            .map(|dt| dt.with_timezone(&Utc));

        assert_eq!(timers[0].next_run, expected);
    }

    #[tokio::test]
    async fn test_disabled_timer_not_due() {
        let mgr = create_test_manager().await;
        // 每秒执行的 timer
        mgr.create_timer("test", "* * * * * *", "prompt", Some("telegram"), "123")
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
        mgr.create_timer("test", "* * * * * *", "prompt", Some("telegram"), "123")
            .await
            .unwrap();

        let before = mgr.list_timers().await;
        assert!(before[0].last_run.is_none());

        mgr.mark_executed("test").await;

        let after = mgr.list_timers().await;
        assert!(after[0].last_run.is_some());
        assert_eq!(after[0].trigger_count, 1);
    }

    #[tokio::test]
    async fn test_list_timers_empty() {
        let mgr = create_test_manager().await;
        assert!(mgr.list_timers().await.is_empty());
    }

    #[tokio::test]
    async fn test_create_timer_with_explicit_channel() {
        let mgr = create_test_manager().await;
        mgr.create_timer("test", "0 * * * * *", "prompt", Some("telegram"), "123")
            .await
            .unwrap();

        let timers = mgr.list_timers().await;
        assert_eq!(timers[0].channel.as_deref(), Some("telegram"));
    }

    #[tokio::test]
    async fn test_create_timer_invalid_chat_id_for_channel() {
        let mgr = create_test_manager().await;
        let result = mgr
            .create_timer("test", "0 * * * * *", "prompt", Some("feishu"), "test-session")
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("chat_id 必须以 'oc_' 开头"));
    }

    #[tokio::test]
    async fn test_create_timer_with_max_trigger_count() {
        let mgr = create_test_manager().await;
        mgr.create_timer_with_limit("test", "* * * * * *", "prompt", Some("telegram"), "123", 3)
            .await
            .unwrap();

        let timers = mgr.list_timers().await;
        assert_eq!(timers[0].max_trigger_count, 3);
        assert_eq!(timers[0].trigger_count, 0);
    }

    #[tokio::test]
    async fn test_create_timer_invalid_max_trigger_count() {
        let mgr = create_test_manager().await;
        let result = mgr
            .create_timer_with_limit("test", "* * * * * *", "prompt", Some("telegram"), "123", 0)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("max_trigger_count 必须为 -1（不限制）或正整数"));
    }

    #[tokio::test]
    async fn test_mark_executed_disables_timer_when_limit_reached() {
        let mgr = create_test_manager().await;
        mgr.create_timer_with_limit("test", "* * * * * *", "prompt", Some("telegram"), "123", 2)
            .await
            .unwrap();

        mgr.mark_executed("test").await;
        mgr.mark_executed("test").await;

        let timer = mgr.list_timers().await.into_iter().next().unwrap();
        assert_eq!(timer.trigger_count, 2);
        assert!(!timer.enabled);
        assert!(timer.next_run.is_none());
    }

    #[tokio::test]
    async fn test_create_timer_requires_channel_or_inferrable_chat_id() {
        let mgr = create_test_manager().await;
        let result = mgr
            .create_timer("test", "* * * * * *", "prompt", None, "test-chat")
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("无法从 chat_id 'test-chat' 推断 channel"));
    }
}
