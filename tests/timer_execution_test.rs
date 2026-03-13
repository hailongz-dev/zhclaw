//! 定时任务执行完整流程测试
//!
//! 此测试套件演示了定时任务的完整生命周期：
//! 1. 创建定时任务
//! 2. 启动调度器
//! 3. 监控任务触发
//! 4. 验证执行结果
//! 5. 清理资源

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Mutex;
use zhclaw::mcp::timer_manager::TimerManager;

/// 定时任务执行跟踪器
struct ExecutionTracker {
    executions: Arc<Mutex<Vec<String>>>,
    count: Arc<AtomicUsize>,
}

impl ExecutionTracker {
    fn new() -> Self {
        Self {
            executions: Arc::new(Mutex::new(Vec::new())),
            count: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn record(&self, timer_name: &str) {
        self.count.fetch_add(1, Ordering::SeqCst);
        let mut execs = self.executions.lock().await;
        execs.push(format!("{}@{:?}", timer_name, Utc::now()));
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    async fn get_executions(&self) -> Vec<String> {
        self.executions.lock().await.clone()
    }
}

/// 测试 1：创建定时任务并验证属性
#[tokio::test]
async fn test_timer_creation_and_properties() {
    println!("\n=== 测试 1：定时任务创建和属性 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );

    // 创建一个每秒执行的定时任务
    let result = mgr
        .create_timer("every_second", "* * * * * *", "echo hello", Some("telegram"), "123")
        .await;

    assert!(result.is_ok(), "定时任务创建应该成功");

    // 验证定时任务属性
    let timers = mgr.list_timers().await;
    assert_eq!(timers.len(), 1, "应该有 1 个定时任务");

    let timer = &timers[0];
    assert_eq!(timer.name, "every_second");
    assert_eq!(timer.cron_expr, "* * * * * *");
    assert_eq!(timer.prompt, "echo hello");
    assert_eq!(timer.chat_id, "123");
    assert_eq!(timer.channel.as_deref(), Some("telegram"));
    assert!(timer.enabled, "新建的定时任务应该是启用状态");
    assert!(timer.next_run.is_some(), "应该计算出下次运行时间");
    assert!(timer.last_run.is_none(), "初始状态下 last_run 应为 None");

    println!("✓ 定时任务创建成功");
    println!("  名称: {}", timer.name);
    println!("  表达式: {}", timer.cron_expr);
    println!("  下次运行: {:?}", timer.next_run);
}

/// 测试 2：定时任务调度和触发
#[tokio::test]
async fn test_timer_scheduling_and_triggering() {
    println!("\n=== 测试 2：定时任务调度和触发 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );
    let tracker = Arc::new(ExecutionTracker::new());

    // 创建每秒执行一次的定时任务
    mgr.create_timer("trigger_test", "* * * * * *", "test prompt", Some("telegram"), "123")
        .await
        .unwrap();

    println!("✓ 已创建定时任务: trigger_test (每秒执行)");

    // 启动调度器
    let tracker_clone = Arc::clone(&tracker);
    let scheduler_handle = mgr.start_scheduler(move |timer| {
        let tracker = Arc::clone(&tracker_clone);
        async move {
            println!("  → 定时任务触发: {} (prompt: {})", timer.name, timer.prompt);
            tracker.record(&timer.name).await;
        }
    });

    // 等待调度器执行几个周期
    println!("⏳ 等待定时任务触发...");
    tokio::time::sleep(Duration::from_secs(4)).await;

    // 验证执行次数（应该至少 3-4 次）
    let exec_count = tracker.count();
    println!("✓ 定时任务已执行 {} 次", exec_count);
    assert!(
        exec_count >= 3,
        "定时任务应该至少执行 3 次，实际 {}",
        exec_count
    );

    // 停止调度器
    scheduler_handle.abort();

    // 输出执行历史
    let executions = tracker.get_executions().await;
    for (i, exec) in executions.iter().enumerate() {
        println!("  {} 次执行: {}", i + 1, exec);
    }
}

/// 测试 3：多个定时任务并发执行
#[tokio::test]
async fn test_multiple_timers_concurrent_execution() {
    println!("\n=== 测试 3：多个定时任务并发执行 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );
    let tracker = Arc::new(ExecutionTracker::new());

    // 创建多个定时任务，不同的执行频率
    mgr.create_timer("fast_timer", "* * * * * *", "fast", Some("telegram"), "123")
        .await
        .unwrap();
    mgr.create_timer("slow_timer", "*/2 * * * * *", "slow", Some("telegram"), "456")
        .await
        .unwrap();

    println!("✓ 已创建 2 个定时任务:");
    println!("  - fast_timer: 每秒执行");
    println!("  - slow_timer: 每 2 秒执行");

    // 启动调度器并跟踪执行
    let tracker_clone = Arc::clone(&tracker);
    let scheduler_handle = mgr.start_scheduler(move |timer| {
        let tracker = Arc::clone(&tracker_clone);
        async move {
            println!(
                "  → [{}] 触发: {} ({})",
                chrono::Local::now().format("%H:%M:%S"),
                timer.name,
                timer.prompt
            );
            tracker.record(&timer.name).await;
        }
    });

    // 运行 5 秒
    println!("⏳ 并发运行 5 秒...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 停止调度器
    scheduler_handle.abort();

    // 统计每个定时任务的执行次数
    let executions = tracker.get_executions().await;
    let fast_count = executions.iter().filter(|e| e.starts_with("fast")).count();
    let slow_count = executions.iter().filter(|e| e.starts_with("slow")).count();

    println!("✓ 执行统计:");
    println!("  - fast_timer 执行: {} 次", fast_count);
    println!("  - slow_timer 执行: {} 次", slow_count);

    // fast_timer 应该比 slow_timer 执行更多次
    assert!(
        fast_count > slow_count,
        "快速定时任务应该执行次数更多"
    );
}

/// 测试 4：禁用定时任务
#[tokio::test]
async fn test_disable_timer_stops_execution() {
    println!("\n=== 测试 4：禁用定时任务 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );
    let tracker = Arc::new(ExecutionTracker::new());

    // 创建定时任务
    mgr.create_timer("disable_test", "* * * * * *", "prompt", Some("telegram"), "123")
        .await
        .unwrap();

    let tracker_clone = Arc::clone(&tracker);
    let scheduler_handle = mgr.start_scheduler(move |timer| {
        let tracker = Arc::clone(&tracker_clone);
        async move {
            tracker.record(&timer.name).await;
        }
    });

    // 运行 2 秒让定时任务执行
    println!("⏳ 定时任务运行中...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let count_before = tracker.count();
    println!("  执行次数（禁用前）: {}", count_before);

    // 禁用定时任务
    println!("⛔ 禁用定时任务...");
    mgr.toggle_timer("disable_test", false).await.unwrap();

    // 再运行 2 秒，定时任务应该不再执行
    tokio::time::sleep(Duration::from_secs(2)).await;
    let count_after = tracker.count();
    println!("  执行次数（禁用后）: {}", count_after);

    // 停止调度器
    scheduler_handle.abort();

    // 禁用后执行次数不应该增加
    println!("✓ 禁用定时任务测试通过");
    assert_eq!(count_after, count_before, "禁用后应该停止执行");
}

/// 测试 5：删除定时任务
#[tokio::test]
async fn test_delete_timer() {
    println!("\n=== 测试 5：删除定时任务 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );

    // 创建 2 个定时任务
    mgr.create_timer("timer1", "* * * * * *", "prompt1", Some("telegram"), "123")
        .await
        .unwrap();
    mgr.create_timer("timer2", "* * * * * *", "prompt2", Some("telegram"), "456")
        .await
        .unwrap();

    println!("✓ 已创建 2 个定时任务");

    let timers = mgr.list_timers().await;
    assert_eq!(timers.len(), 2);

    // 删除一个定时任务
    println!("🗑️  删除 timer1...");
    let result = mgr.delete_timer("timer1").await;
    assert!(result.is_ok());

    // 验证只剩一个定时任务
    let timers = mgr.list_timers().await;
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0].name, "timer2");

    println!("✓ 删除定时任务成功，剩余定时任务: {}", timers[0].name);
}

/// 测试 6：定时任务执行时间更新
#[tokio::test]
async fn test_timer_execution_time_updates() {
    println!("\n=== 测试 6：定时任务执行时间更新 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );

    // 创建定时任务
    mgr.create_timer("time_update_test", "* * * * * *", "prompt", Some("telegram"), "123")
        .await
        .unwrap();

    // 获取初始状态
    let timers_before = mgr.list_timers().await;
    let before_last_run = timers_before[0].last_run;
    let before_next_run = timers_before[0].next_run;

    println!("✓ 定时任务初始状态:");
    println!("  - last_run: {:?}", before_last_run);
    println!("  - next_run: {:?}", before_next_run);

    // 等待 1.5 秒确保时间跨越到下一个秒
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // 标记定时任务已执行
    println!("🔄 执行定时任务...");
    mgr.mark_executed("time_update_test").await;

    // 获取更新后的状态
    let timers_after = mgr.list_timers().await;
    let after_last_run = timers_after[0].last_run;
    let after_next_run = timers_after[0].next_run;

    println!("✓ 定时任务执行后状态:");
    println!("  - last_run: {:?}", after_last_run);
    println!("  - next_run: {:?}", after_next_run);

    // 验证时间已更新
    assert!(before_last_run.is_none());
    assert!(after_last_run.is_some(), "last_run 应该被更新");
    
    assert_ne!(
        before_next_run, after_next_run,
        "next_run 应该被更新到下一个执行时刻"
    );

    println!("✓ 定时任务执行时间更新测试通过");
}

/// 测试 7：Cron 表达式验证
#[tokio::test]
async fn test_cron_expression_validation() {
    println!("\n=== 测试 7：Cron 表达式验证 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );

    // 测试有效的 Cron 表达式
    let valid_expressions = vec![
        ("* * * * * *", "每秒"),
        ("0 * * * * *", "每分钟"),
        ("0 0 * * * *", "每小时"),
        ("0 0 0 * * *", "每天"),
        ("0 0 9-17 * * MON-FRI", "工作日 9-17 点"),
    ];

    println!("✓ 测试有效的 Cron 表达式:");
    for (expr, desc) in valid_expressions {
        let name = format!("cron_test_{}", expr.replace(" ", "_"));
        let result = mgr
            .create_timer(&name, expr, "prompt", Some("telegram"), "123")
            .await;
        assert!(
            result.is_ok(),
            "有效的 Cron 表达式应该创建成功: {} ({})",
            expr,
            desc
        );
        println!("  ✓ {} ({})", expr, desc);
    }

    // 测试无效的 Cron 表达式
    println!("\n✓ 测试无效的 Cron 表达式:");
    let invalid_expressions = vec![
        ("invalid", "完全无效"),
        ("60 * * * * *", "秒数超出范围"),
        ("* 60 * * * *", "分钟数超出范围"),
    ];

    for (expr, desc) in invalid_expressions {
        let result = mgr
            .create_timer("invalid_test", expr, "prompt", Some("telegram"), "123")
            .await;
        assert!(
            result.is_err(),
            "无效的 Cron 表达式应该被拒绝: {} ({})",
            expr,
            desc
        );
        println!("  ✓ {} 被正确拒绝 ({})", expr, desc);
    }

    println!("✓ Cron 表达式验证测试通过");
}

/// 集成测试：完整的定时任务工作流
#[tokio::test]
async fn test_complete_timer_workflow() {
    println!("\n=== 集成测试：完整的定时任务工作流 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );
    let tracker = Arc::new(ExecutionTracker::new());

    println!("1️⃣  创建定时任务...");
    mgr.create_timer("workflow_test", "* * * * * *", "Daily task", Some("telegram"), "123")
        .await
        .unwrap();
    println!("   ✓ 定时任务已创建");

    println!("2️⃣  启动调度器...");
    let tracker_clone = Arc::clone(&tracker);
    let scheduler = mgr.start_scheduler(move |timer| {
        let tracker = Arc::clone(&tracker_clone);
        async move {
            tracker.record(&timer.name).await;
        }
    });
    println!("   ✓ 调度器已启动");

    println!("3️⃣  运行定时任务...");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let exec_count = tracker.count();
    println!("   ✓ 定时任务已执行 {} 次", exec_count);

    println!("4️⃣  禁用定时任务...");
    mgr.toggle_timer("workflow_test", false).await.unwrap();
    let disabled_count = tracker.count();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        tracker.count(),
        disabled_count,
        "禁用后不应有新的执行"
    );
    println!("   ✓ 定时任务已禁用");

    println!("5️⃣  重新启用定时任务...");
    mgr.toggle_timer("workflow_test", true).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        tracker.count() > disabled_count,
        "重新启用后应该继续执行"
    );
    println!("   ✓ 定时任务已重新启用");

    println!("6️⃣  清理资源...");
    scheduler.abort();
    mgr.delete_timer("workflow_test").await.unwrap();
    println!("   ✓ 资源已清理");

    println!("\n✅ 完整工作流测试通过!");
    println!(
        "总执行次数: {} 次",
        tracker.count()
    );
}

/// 测试 8：最大触发次数限制
#[tokio::test]
async fn test_timer_max_trigger_count_limit() {
    println!("\n=== 测试 8：最大触发次数限制 ===");

    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    let db_path = temp_file.path().to_string_lossy().to_string();
    drop(temp_file);
    let mgr = Arc::new(
        TimerManager::new_with_db(&db_path)
            .await
            .expect("Failed to create timer manager")
    );
    let tracker = Arc::new(ExecutionTracker::new());

    mgr.create_timer_with_limit("limited_timer", "* * * * * *", "prompt", Some("telegram"), "123", 2)
        .await
        .unwrap();

    let tracker_clone = Arc::clone(&tracker);
    let scheduler = mgr.start_scheduler(move |timer| {
        let tracker = Arc::clone(&tracker_clone);
        async move {
            tracker.record(&timer.name).await;
        }
    });

    tokio::time::sleep(Duration::from_secs(4)).await;
    scheduler.abort();

    let timers = mgr.list_timers().await;
    let timer = timers.iter().find(|t| t.name == "limited_timer").unwrap();

    println!("✓ 触发次数: {}", timer.trigger_count);
    assert_eq!(timer.trigger_count, 2, "应在达到上限后停止");
    assert!(!timer.enabled, "达到上限后应自动禁用");
    assert!(timer.next_run.is_none(), "达到上限后不应再有下次执行时间");
}
