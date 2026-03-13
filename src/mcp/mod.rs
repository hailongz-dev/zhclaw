pub mod timer_manager;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tracing::info;

use crate::executor::process_registry::ProcessRegistry;
use crate::mcp::timer_manager::TimerManager;

/// ZhcLaw MCP Server
#[derive(Clone)]
pub struct ZhclawMcpServer {
    timer_manager: Arc<TimerManager>,
    process_registry: Arc<ProcessRegistry>,
}

impl ZhclawMcpServer {
    pub fn new(
        timer_manager: Arc<TimerManager>,
        process_registry: Arc<ProcessRegistry>,
    ) -> Self {
        Self {
            timer_manager,
            process_registry,
        }
    }

    /// 获取 timer manager
    pub fn timer_manager(&self) -> &Arc<TimerManager> {
        &self.timer_manager
    }

    /// 获取 process registry
    pub fn process_registry(&self) -> &Arc<ProcessRegistry> {
        &self.process_registry
    }
}

/// HTTP 请求结构
#[derive(Debug, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// 启动 MCP HTTP 服务
pub async fn serve_http(
    addr: &str,
    server: ZhclawMcpServer,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("启动 MCP Server HTTP 服务在 http://{}/mcp", addr);

    let app = Router::new()
        .route("/mcp", post(handle_mcp_request))
        .route("/mcp", get(handle_mcp_get))
        .route("/mcp", axum::routing::delete(handle_mcp_delete))
        .with_state(server);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// 处理 MCP POST 请求
pub async fn handle_mcp_request(
    axum::extract::State(server): axum::extract::State<ZhclawMcpServer>,
    Json(request): Json<McpRequest>,
) -> Json<McpResponse> {
    info!("收到 MCP 请求: method={}", request.method);

    match request.method.as_str() {
        "initialize" => handle_initialize(request.id).await,
        "tools/list" => handle_tools_list(&server, request.id).await,
        "tools/call" => handle_tools_call(&server, request.id, request.params).await,
        _ => Json(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: None,
            error: Some(McpError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        }),
    }
}

/// 处理 GET 请求（用于 SSE 或健康检查）
async fn handle_mcp_get() -> &'static str {
    "MCP Server is running"
}

/// 处理 DELETE 请求
async fn handle_mcp_delete() -> &'static str {
    "MCP Server DELETE endpoint"
}

/// 处理 initialize 请求
async fn handle_initialize(id: Option<serde_json::Value>) -> Json<McpResponse> {
    let result = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "zhclaw",
            "version": env!("CARGO_PKG_VERSION")
        }
    });

    Json(McpResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    })
}

/// 处理 tools/list 请求
async fn handle_tools_list(
    _server: &ZhclawMcpServer,
    id: Option<serde_json::Value>,
) -> Json<McpResponse> {
    // 获取所有工具信息
    let tools = vec![
        json!({
            "name": "create_timer",
            "description": "创建定时任务，按 cron 表达式周期性触发 agent 命令",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "定时任务名称"},
                    "cron_expr": {"type": "string", "description": "Cron 表达式，如 '0 */30 * * * *'"},
                    "prompt": {"type": "string", "description": "要执行的 prompt"},
                    "chat_id": {"type": "string", "description": "结果发送到的 chat_id"}
                },
                "required": ["name", "cron_expr", "prompt", "chat_id"]
            }
        }),
        json!({
            "name": "list_timers",
            "description": "列出所有定时任务",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "delete_timer",
            "description": "删除指定定时任务",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "定时任务名称"}
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "toggle_timer",
            "description": "暂停或恢复定时任务",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "定时任务名称"},
                    "enabled": {"type": "boolean", "description": "是否启用"}
                },
                "required": ["name", "enabled"]
            }
        }),
        json!({
            "name": "list_processes",
            "description": "列出当前所有运行中的 agent 命令行进程",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "kill_process",
            "description": "终止指定的运行中进程",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "process_id": {"type": "string", "description": "进程 ID"}
                },
                "required": ["process_id"]
            }
        }),
    ];

    Json(McpResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(json!({ "tools": tools })),
        error: None,
    })
}

/// 处理 tools/call 请求
async fn handle_tools_call(
    server: &ZhclawMcpServer,
    id: Option<serde_json::Value>,
    params: Option<serde_json::Value>,
) -> Json<McpResponse> {
    let params = match params {
        Some(p) => p,
        None => {
            return Json(McpResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(McpError {
                    code: -32602,
                    message: "Invalid params".to_string(),
                    data: None,
                }),
            });
        }
    };

    let tool_name = params.get("name").and_then(|v| v.as_str());
    let arguments = params.get("arguments");

    match tool_name {
        Some("create_timer") => {
            let empty_args = json!({});
            let args = arguments.unwrap_or(&empty_args);
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let cron_expr = args.get("cron_expr").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let chat_id = args.get("chat_id").and_then(|v| v.as_str()).unwrap_or("").to_string();

            match server.timer_manager.create_timer(&name, &cron_expr, &prompt, &chat_id).await {
                Ok(_) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({
                        "content": [{"type": "text", "text": format!("定时任务 '{}' 创建成功", name)}]
                    })),
                    error: None,
                }),
                Err(e) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("创建失败: {}", e),
                        data: None,
                    }),
                }),
            }
        }
        Some("list_timers") => {
            let timers = server.timer_manager.list_timers().await;
            let json_str = serde_json::to_string_pretty(&timers).unwrap_or_default();
            Json(McpResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({
                    "content": [{"type": "text", "text": json_str}]
                })),
                error: None,
            })
        }
        Some("delete_timer") => {
            let empty_args = json!({});
            let args = arguments.unwrap_or(&empty_args);
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();

            match server.timer_manager.delete_timer(&name).await {
                Ok(_) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({
                        "content": [{"type": "text", "text": format!("定时任务 '{}' 已删除", name)}]
                    })),
                    error: None,
                }),
                Err(e) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("删除失败: {}", e),
                        data: None,
                    }),
                }),
            }
        }
        Some("toggle_timer") => {
            let empty_args = json!({});
            let args = arguments.unwrap_or(&empty_args);
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let enabled = args.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

            match server.timer_manager.toggle_timer(&name, enabled).await {
                Ok(_) => {
                    let state = if enabled { "启用" } else { "禁用" };
                    Json(McpResponse {
                        jsonrpc: "2.0".to_string(),
                        id,
                        result: Some(json!({
                            "content": [{"type": "text", "text": format!("定时任务 '{}' 已{}", name, state)}]
                        })),
                        error: None,
                    })
                }
                Err(e) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("操作失败: {}", e),
                        data: None,
                    }),
                }),
            }
        }
        Some("list_processes") => {
            let processes = server.process_registry.list_running().await;
            let json_str = serde_json::to_string_pretty(&processes).unwrap_or_default();
            Json(McpResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({
                    "content": [{"type": "text", "text": json_str}]
                })),
                error: None,
            })
        }
        Some("kill_process") => {
            let empty_args = json!({});
            let args = arguments.unwrap_or(&empty_args);
            let process_id = args.get("process_id").and_then(|v| v.as_str()).unwrap_or("").to_string();

            match server.process_registry.kill_process(&process_id).await {
                Ok(_) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({
                        "content": [{"type": "text", "text": format!("进程 '{}' 已终止", process_id)}]
                    })),
                    error: None,
                }),
                Err(e) => Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("终止失败: {}", e),
                        data: None,
                    }),
                }),
            }
        }
        _ => Json(McpResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(McpError {
                code: -32601,
                message: format!("Tool not found: {:?}", tool_name),
                data: None,
            }),
        }),
    }
}
