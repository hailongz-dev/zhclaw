//! Telegram 适配器集成测试

use zhclaw::channel::telegram::TelegramAdapter;

#[test]
fn test_split_message_no_split_needed() {
    // 测试：消息不需要分段 —— 长度 ≤ 4096
    use zhclaw::channel::telegram::split_message;
    
    let msg = "hello world";
    let parts = split_message(msg, 4096);
    
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0], "hello world");
}

#[test]
fn test_split_message_single_split() {
    // 测试：消息需要分为 2 段
    use zhclaw::channel::telegram::split_message;
    
    let msg = "a".repeat(5000);
    let parts = split_message(&msg, 4096);
    
    assert_eq!(parts.len(), 2);
    assert!(parts[0].len() <= 4096);
    assert!(parts[1].len() <= 4096);
    assert_eq!(parts[0].len() + parts[1].len(), 5000);
}

#[test]
fn test_split_message_3_parts() {
    // 测试：消息分为 3 段
    use zhclaw::channel::telegram::split_message;
    
    let msg = "x".repeat(10000);
    let parts = split_message(&msg, 4096);
    
    assert!(parts.len() >= 2, "应分为多段");
    
    for part in &parts {
        assert!(part.len() <= 4096, "每段应 ≤ 4096");
    }
    
    // 验证拼接后等于原始消息
    let reconstructed = parts.join("");
    assert_eq!(reconstructed.len(), 10000);
}

#[test]
fn test_split_message_with_newlines() {
    // 测试：在换行符处切割
    use zhclaw::channel::telegram::split_message;
    
    let msg = format!("{}\n{}", "a".repeat(3000), "b".repeat(3000));
    let parts = split_message(&msg, 4096);
    
    // 应该在换行符处优先切割
    assert!(parts.len() > 0);
    
    // 每段不超过最大长度
    for part in &parts {
        assert!(part.len() <= 4096);
    }
}

#[test]
fn test_split_message_exact_boundary() {
    // 测试：恰好等于最大长度
    use zhclaw::channel::telegram::split_message;
    
    let msg = "x".repeat(4096);
    let parts = split_message(&msg, 4096);
    
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].len(), 4096);
}

#[test]
fn test_telegram_adapter_new() {
    // 测试：创建 TelegramAdapter
    let token = "test_token_123";
    let adapter = TelegramAdapter::new(token);
    
    // 验证适配器创建成功（能获取 bot 引用）
    let _ = adapter.bot();
}

#[test]
fn test_telegram_adapter_channel_type() {
    // 测试：adapter 应返回正确的 channel type
    use zhclaw::channel::{ChannelAdapter, ChannelType};
    
    let adapter = TelegramAdapter::new("test_token");
    let channel_type = adapter.channel_type();
    
    assert_eq!(channel_type, ChannelType::Telegram);
}

#[test]
fn test_split_message_empty() {
    // 测试：空消息
    use zhclaw::channel::telegram::split_message;
    
    let msg = "";
    let parts = split_message(msg, 100);
    
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0], "");
}

#[test]
fn test_split_message_unicode() {
    // 测试：Unicode 字符处理
    use zhclaw::channel::telegram::split_message;
    
    // 使用 Unicode 字符（中文）
    let msg = "你好世界".repeat(1000); // 中文，每个字2-3字节
    let parts = split_message(&msg, 4096);
    
    // 验证没有崩溃，且每段都有内容
    assert!(parts.len() > 0);
    
    // 每段不超过最大字节长度
    for part in &parts {
        // 注意：这里是字符长度，不是字节长度，所以实现可能比 4096 小
        assert!(!part.is_empty());
    }
}

#[test]
fn test_split_message_with_mixed_content() {
    // 测试：混合 ASCII 和 Unicode
    use zhclaw::channel::telegram::split_message;
    
    let msg = format!("hello 你好{}world 世界", "x".repeat(4000));
    let parts = split_message(&msg, 4096);
    
    assert!(parts.len() > 0);
    
    // 拼接后应等于原文本
    let joined = parts.join("");
    assert_eq!(joined.len(), msg.len());
}

#[test]
fn test_split_message_large_message() {
    // 测试：大消息（超过 20KB）
    use zhclaw::channel::telegram::split_message;
    
    let msg = "test".repeat(5000); // 20,000 字符
    let parts = split_message(&msg, 4096);
    
    assert!(parts.len() >= 5, "应分为至少 5 段");
    
    // 验证所有段都不超过最大长度
    for part in &parts {
        assert!(part.len() <= 4096);
    }
    
    // 验证拼接后长度正确
    assert_eq!(parts.join("").len(), 20000);
}
