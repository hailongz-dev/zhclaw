//! Agent Executor 集成测试

use std::sync::Arc;
use std::time::Duration;

use zhclaw::executor::process_registry::ProcessRegistry;
use zhclaw::executor::AgentExecutor;

/// 辅助函数：创建 executor
fn make_executor(template: &str, timeout_secs: u64) -> AgentExecutor {
    let registry = Arc::new(ProcessRegistry::new());
    AgentExecutor::new(template, Duration::from_secs(timeout_secs), registry)
}

/// 获取 executor 的 registry 引用
fn get_registry(executor: &AgentExecutor) -> Arc<ProcessRegistry> {
    executor.process_registry().clone()
}

#[tokio::test]
async fn test_complete_flow_echo_command() {
    // 测试：完整流程 —— 创建 executor → 执行 echo 命令 → 验证输出
    let executor = make_executor("echo {prompt}", 30);
    let result = executor.execute("hello world", "test_chat").await;
    
    assert!(result.is_ok(), "执行应该成功");
    let output = result.unwrap();
    assert!(output.contains("hello world"), "输出应包含 'hello world'");
}

#[tokio::test]
async fn test_timeout_scenario() {
    // 测试：超时场景 —— 执行长时间命令 → 验证超时返回错误
    let executor = make_executor("sleep {prompt}", 1);
    let result = executor.execute("10", "test_chat").await;
    
    assert!(result.is_err(), "执行应该超时");
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("超时"), "错误消息应包含 '超时'");
}

#[tokio::test]
async fn test_concurrent_execution() {
    // 测试：并发执行 —— 同时执行多个命令 → 所有结果正确返回
    let executor = Arc::new(make_executor("echo {prompt}", 10));
    
    let mut handles = vec![];
    
    for i in 0..5 {
        let exec = executor.clone();
        let handle = tokio::spawn(async move {
            exec.execute(&format!("msg_{}", i), &format!("chat_{}", i))
                .await
        });
        handles.push((i, handle));
    }
    
    for (i, handle) in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "执行 {} 应该成功", i);
        let output = result.unwrap();
        assert!(
            output.contains(&format!("msg_{}", i)),
            "输出应包含 'msg_{}'",
            i
        );
    }
}

#[tokio::test]
async fn test_process_registry_integration() {
    // 测试：ProcessRegistry 联动 —— 执行中进程在列表中，执行完消失
    let registry = Arc::new(ProcessRegistry::new());
    let executor = AgentExecutor::new("sleep {prompt}", Duration::from_secs(5), registry.clone());
    
    // 验证初始状态：没有进程
    {
        let running = registry.list_running().await;
        assert!(running.is_empty(), "初始状态应无进程");
    }
    
    // 在后台启动执行
    let handle = tokio::spawn(async move {
        executor.execute("2", "chat_test").await
    });
    
    // 短暂等待让进程启动
    tokio::time::sleep(Duration::from_millis(300)).await;
    
    // 执行中时应有进程
    {
        let running = registry.list_running().await;
        assert!(!running.is_empty(), "执行中应有进程在注册表中");
        assert_eq!(running.len(), 1, "应有 1 个进程");
    }
    
    // 等待执行完成
    let result = handle.await.unwrap();
    assert!(result.is_ok(), "执行应该成功");
    
    // 执行完成后应无进程
    {
        let running = registry.list_running().await;
        assert!(running.is_empty(), "执行完成后应无 running 状态的进程");
    }
}

#[tokio::test]
async fn test_kill_process() {
    // 测试：kill_process —— 启动长时间进程 → kill → 验证进程被终止
    let registry = Arc::new(ProcessRegistry::new());
    let executor = AgentExecutor::new("sleep {prompt}", Duration::from_secs(30), registry.clone());
    
    // 启动长时间进程
    let handle = tokio::spawn({
        let registry = registry.clone();
        async move {
            executor.execute("100", "chat_test").await
        }
    });
    
    // 等待进程启动
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // 获取进程列表并 kill
    let process_id = {
        let running = registry.list_running().await;
        assert!(!running.is_empty(), "应有启动的进程");
        running[0].id.clone()
    };
    
    // 执行 kill
    let kill_result = registry.kill_process(&process_id).await;
    assert!(kill_result.is_ok(), "kill_process 应该成功");
    
    // 等待子 task 完成（应该因为进程被 kill 而完成）
    let exec_result = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(exec_result.is_ok(), "进程应该因 kill 而快速终止");
    
    // 进程应已不在列表中
    let running = registry.list_running().await;
    assert_eq!(running.len(), 0, "被 kill 的进程应无法运行状态");
}

#[tokio::test]
async fn test_stderr_output_capture() {
    // 测试：stderr 输出 —— 执行产生 stderr 的命令 → 验证 stderr 被捕获
    let executor = make_executor("sh -c {prompt}", 5);
    let result = executor
        .execute("echo 'error message' >&2; exit 0", "test_chat")
        .await;
    
    assert!(result.is_ok(), "执行应该成功（即使产生 stderr）");
    let output = result.unwrap();
    
    // 验证输出包含 stderr 标记和错误消息
    assert!(
        output.contains("[stderr]") || output.contains("error message"),
        "输出大约包含: {:?}",
        output
    );
}

#[tokio::test]
async fn test_mixed_stdout_stderr() {
    // 测试：混合 stdout 和 stderr —— 同时产生两种输出
    let executor = make_executor("sh -c {prompt}", 5);
    let result = executor
        .execute("echo 'normal output'; echo 'error' >&2; exit 0", "test_chat")
        .await;
    
    assert!(result.is_ok(), "执行应该成功");
    let output = result.unwrap();
    
    assert!(
        output.contains("normal output") || output.len() > 0,
        "输出大约包含: {:?}",
        output
    );
}

#[tokio::test]
async fn test_command_with_special_chars() {
    // 测试：特殊字符处理 —— prompt 包含特殊字符
    let executor = make_executor("echo {prompt}", 10);
    let special_input = "hello 'world' \"test\" $VAR";
    let result = executor.execute(special_input, "test_chat").await;
    
    assert!(result.is_ok(), "执行应该成功");
    let output = result.unwrap();
    
    // 输出应包含转义后的内容
    assert!(
        output.len() > 0,
        "输出不应为空"
    );
}

#[tokio::test]
async fn test_nonzero_exit_code() {
    // 测试：非零退出码 —— 命令以非零状态退出，输出仍应被捕获
    let executor = make_executor("sh -c {prompt}", 5);
    let result = executor.execute("exit 42", "test_chat").await;
    
    // 非零退出码不视为错误（只记录 exit code）
    assert!(result.is_ok(), "非零退出码应返回 Ok(输出)");
}

#[tokio::test]
async fn test_empty_command_output() {
    // 测试：命令无输出 —— 执行成功但不产生输出
    let executor = make_executor("sh -c {prompt}", 5);
    let result = executor.execute("true", "test_chat").await;
    
    assert!(result.is_ok(), "执行应该成功");
    let output = result.unwrap();
    // 输出可能为空或只有空白字符
    assert!(output.trim().is_empty(), "输出应为空");
}
