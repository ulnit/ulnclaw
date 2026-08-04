# 集成指南

如何将 ulnclaw 集成到你的 Rust 项目中。

## 目录

- [基础集成](#基础集成)
- [Web 服务集成](#web-服务集成)
- [CLI 应用程序](#cli-应用程序)
- [嵌入式系统](#嵌入式系统)
- [ZStep 集成](#zstep-集成)
- [高级模式](#高级模式)
- [最佳实践](#最佳实践)

## 基础集成

### 步骤 1：添加依赖

在 `Cargo.toml` 中添加 ulnclaw：

```toml
[dependencies]
ulnclaw = { git = "https://gitee.com/ushaw/ulnclaw.git", branch = "master" }
tokio = { version = "1", features = ["full"] }
serde_json = "1.0"
```

或使用本地路径进行开发：

```toml
[dependencies]
ulnclaw = { path = "../ulnclaw" }
```

### 步骤 2：创建提供商

```rust
use ulnclaw::prelude::*;

let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key("sk-...")  // 在生产环境中使用环境变量
    .model("gpt-4")
    .build()?;
```

### 步骤 3：定义工具

```rust
let mut tools = ToolRegistry::new();

// 简单工具
tools.register(tool("get_weather")
    .description("获取城市天气")
    .parameters(json!({
        "type": "object",
        "properties": {
            "city": {"type": "string"}
        },
        "required": ["city"]
    }))
    .handler(|args| async move {
        let city = args["city"].as_str().unwrap_or("未知");
        Ok(json!({
            "city": city,
            "temperature": "22°C",
            "condition": "晴朗"
        }))
    })
    .toolset("weather")
    .build()?);
```

### 步骤 4：创建并运行代理

```rust
let agent = Agent::new(Arc::new(provider), tools)
    .with_config(AgentConfig {
        system_prompt: Some("你是一个有帮助的天气助手。".into()),
        max_iterations: 50,
        ..Default::default()
    });

let result = agent.run("北京天气怎么样？", None).await?;
println!("{}", result.content);
```

## Web 服务集成

### 内置 HTTP 网关（推荐）

ulnclaw 自带 OpenAI 兼容网关 —— 无需包装代码。任何 OpenAI 兼容前端
（Open WebUI、LobeChat、LibreChat、NextChat、ChatBox……）只需指向
`http://host:port/v1` 即可接入。

```bash
# config.toml
[gateway]
host = "127.0.0.1"
port = 8642
key = "sk-..."                 # 可选 bearer 令牌（ULNCLAW_GATEWAY_KEY）

# 启动
ulnclaw gateway
```

```rust
// ……或嵌入你自己的二进制：
let router = ulnclaw::gateway::ApprovalRouter::new();
// 在 agent 的工具上下文上安装审批回调，把确认级命令路由进 run
// （完整接线见 main.rs 的 gateway_cmd），然后：
let state = ulnclaw::gateway::GatewayState::new(
    agent,                       // 已附加 SQLite 存储的 Arc<Agent>
    "my-agent".to_string(),      // 对外模型名
    "openai".to_string(),        // provider 标签
    Some("sk-...".to_string()),  // bearer 密钥（None = 开放）
    router,                      // 运行审批 router
)?;
ulnclaw::gateway::serve(state, "127.0.0.1", 8642).await?;
```

端点：`/v1/chat/completions`（`X-Ulnclaw-Session-Id` 会话续接）、
`/v1/responses`、`/v1/models`、`/v1/capabilities`、`/api/sessions`
增删查改 + patch/fork + 会话级模型锁 + 会话内聊天、`/api/jobs` 定时任务
管理（增删查改 + pause/resume/run）、`/v1/skills` 与 `/v1/toolsets`
发现端点、`/api/model/options`、`/v1/runs` 异步运行（SSE 事件 +
运行审批 `POST /v1/runs/:id/approval`）。完整列表见
[API 参考](api-reference.md#http-网关-gateway)。


### Axum 示例

```rust
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use ulnclaw::prelude::*;

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    session_id: Option<String>,
}

#[derive(Serialize)]
struct ChatResponse {
    reply: String,
    usage: Usage,
}

async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    let result = state.agent.run(&req.message, None).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Json(ChatResponse {
        reply: result.content,
        usage: result.usage,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key(&std::env::var("OPENAI_API_KEY").unwrap())
        .model("gpt-4")
        .build()?;

    let tools = ToolRegistry::new();
    let agent = Agent::new(Arc::new(provider), tools);

    let state = AppState {
        agent: Arc::new(agent),
    };

    let app = Router::new()
        .route("/chat", post(chat_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
    
    Ok(())
}
```

### 带会话持久化

```rust
use ulnclaw::session::{MemorySessionStore, SessionStore};

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    sessions: Arc<MemorySessionStore>,
}

async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    // 加载或创建会话
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let history = state.sessions.load_session(&session_id)
        .ok()
        .flatten()
        .map(|s| s.messages);

    // 运行代理
    let result = state.agent.run(&req.message, history).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 保存会话
    let mut session = ulnclaw::session::new_session(&session_id);
    session.messages = result.conversation.clone();
    state.sessions.save_session(&session).ok();

    Ok(Json(ChatResponse {
        reply: result.content,
        usage: result.usage,
    }))
}
```

## CLI 应用程序

### 交互式 REPL

```rust
use ulnclaw::prelude::*;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<()> {
    let provider = OpenAiProvider::builder()
        .endpoint("https://api.openai.com/v1")
        .api_key(&std::env::var("OPENAI_API_KEY").unwrap())
        .model("gpt-4")
        .build()?;

    let tools = ToolRegistry::new();
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some("你是一个有帮助的助手。".into()),
            ..Default::default()
        });

    println!("ulnclaw CLI - 输入 'exit' 退出\n");

    let mut history = Vec::new();
    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        if input == "exit" {
            break;
        }

        match agent.run(input, Some(history.clone())).await {
            Ok(result) => {
                println!("\n{}\n", result.content);
                history = result.conversation;
            }
            Err(e) => {
                eprintln!("错误：{}\n", e);
            }
        }
    }

    Ok(())
}
```

## 嵌入式系统

### 资源受限环境

```rust
use ulnclaw::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // 为嵌入式系统使用更小的模型
    let provider = OpenAiProvider::builder()
        .endpoint("http://localhost:11434/v1")  // 本地 Ollama
        .model("qwen2.5:1.5b")  // 小模型
        .max_tokens(1024)  // 限制令牌
        .build()?;

    // 最小工具集
    let mut tools = ToolRegistry::new();
    tools.register(tool("read_sensor")
        .description("读取传感器值")
        .handler(|args| async move {
            let sensor = args["sensor"].as_str().unwrap();
            // 从硬件读取
            Ok(json!({"value": 42.0, "unit": "°C"}))
        })
        .build()?);

    // 保守配置
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            max_iterations: 10,  // 更少的迭代
            strip_thinking_blocks: true,
            ..Default::default()
        });

    let result = agent.run("读取温度传感器", None).await?;
    println!("{}", result.content);

    Ok(())
}
```

## ZStep 集成

### 替换手工代理循环

ulnclaw 旨在驱动 ZStep AI 助手。以下是集成方法：

#### 之前（手工实现）

```rust
// zstep-api/src/lib.rs 中的旧方法
async fn agent_chat(
    State(state): State<ApiState>,
    Json(request): Json<AgentChatRequest>,
) -> Result<Json<AgentChatResponse>> {
    // 手动提供商选择
    // 手动工具调用循环
    // 手动消息格式化
    // 手动错误处理
    // ...
}
```

#### 之后（ulnclaw）

```rust
use ulnclaw::prelude::*;
use crate::agent_bridge::{build_agent, convert_history};

async fn agent_chat(
    State(state): State<ApiState>,
    Json(request): Json<AgentChatRequest>,
) -> Result<Json<AgentChatResponse>> {
    // 从 ZStep 配置构建提供商
    let provider_config = get_provider_config(&state).await?;
    let provider = provider_config.build()?;

    // 从 ZStep MCP 工具构建工具
    let tools = build_zstep_tools(&state).await;

    // 创建代理
    let agent = Agent::new(Arc::new(provider), tools)
        .with_config(AgentConfig {
            system_prompt: Some(get_system_prompt()),
            max_iterations: 50,
            ..Default::default()
        });

    // 转换历史
    let history = request.conversation_id
        .and_then(|id| load_history(&state, &id))
        .map(|h| convert_history(&h));

    // 运行代理
    let result = agent.run(&request.message, history).await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(AgentChatResponse {
        reply: result.content,
        tools_called: result.tool_calls.into_iter().map(|tc| {
            ToolCallInfo {
                name: tc.name,
                arguments: tc.arguments,
                result: tc.result,
            }
        }).collect(),
        usage: result.usage,
    }))
}
```

### 桥接模块

在 zstep-api 中创建 `src/agent_bridge.rs`：

```rust
use crate::ApiState;
use ulnclaw::{
    Agent, AgentConfig,
    provider::{Message, ProviderConfig, ProviderKind},
    tools::{tool, ToolRegistry},
};
use std::sync::Arc;

pub fn build_zstep_tools(state: &ApiState) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    
    // 注册所有 ZStep MCP 工具
    register_connection_tools(&mut registry, state);
    register_task_tools(&mut registry, state);
    register_run_tools(&mut registry, state);
    register_system_tools(&mut registry, state);
    
    registry
}

fn register_connection_tools(registry: &mut ToolRegistry, state: &ApiState) {
    // 使用宏创建调用 execute_agent_tool 的工具
    macro_rules! zstep_tool {
        ($name:expr, $desc:expr, $params:expr) => {{
            let state_clone = state.clone();
            tool($name)
                .description($desc)
                .parameters($params)
                .handler(move |args| {
                    let state = state_clone.clone();
                    let name = $name.to_string();
                    async move {
                        execute_agent_tool(&state, &name, &args).await
                            .map_err(|e| ulnclaw::AgentError::tool(e))
                    }
                })
                .build()
        }};
    }

    registry.register(zstep_tool!(
        "list_connections",
        "列出所有连接资产",
        json!({"type": "object", "properties": {}})
    ).unwrap());

    // ... 更多工具
}
```

## 高级模式

### 流式响应

```rust
use futures::StreamExt;

let agent = Agent::new(Arc::new(provider), tools)
    .with_callbacks(AgentCallbacks {
        on_stream_delta: Some(Box::new(|delta| {
            print!("{}", delta);
            std::io::stdout().flush().unwrap();
        })),
        ..Default::default()
    });
```

### 工具进度追踪

```rust
let agent = Agent::new(Arc::new(provider), tools)
    .with_callbacks(AgentCallbacks {
        on_tool_start: Some(Box::new(|name, args| {
            println!("🔧 执行中：{}", name);
        })),
        on_tool_complete: Some(Box::new(|name, result| {
            println!("✓ 完成：{}", name);
        })),
        ..Default::default()
    });
```

### 多提供商回退

```rust
use ulnclaw::provider::ProviderConfig;

let providers = vec![
    ProviderConfig {
        name: "主要".into(),
        kind: ProviderKind::OpenAiCompatible,
        endpoint: "https://api.openai.com/v1".into(),
        api_key: Some("sk-primary".into()),
        model: "gpt-4".into(),
        ..Default::default()
    },
    ProviderConfig {
        name: "回退".into(),
        kind: ProviderKind::OpenAiCompatible,
        endpoint: "https://dashscope.aliyuncs.com/compatible-mode".into(),
        api_key: Some("sk-fallback".into()),
        model: "qwen-plus".into(),
        ..Default::default()
    },
];

let mut last_error = None;
for config in providers {
    match config.build() {
        Ok(provider) => {
            let agent = Agent::new(Arc::from(provider), tools.clone());
            match agent.run(&message, None).await {
                Ok(result) => return Ok(result),
                Err(e) => last_error = Some(e),
            }
        }
        Err(e) => last_error = Some(e),
    }
}

Err(last_error.unwrap())
```

### 自定义上下文管理

```rust
use ulnclaw::context::PromptBuilder;

let system_prompt = PromptBuilder::new()
    .identity("你是 ZStep AI，一个数据同步助手。")
    .tool_guidance("使用工具管理连接、任务和运行。")
    .add_skill("在执行破坏性操作前始终确认。")
    .add_context_file(include_str!("zstep_context.md"))
    .add_env_hint("platform", "ZStep v1.0")
    .memory("用户是经验丰富的数据工程师。")
    .build();

let agent = Agent::new(Arc::new(provider), tools)
    .with_config(AgentConfig {
        system_prompt: Some(system_prompt),
        ..Default::default()
    });
```

## 最佳实践

### 1. 使用环境变量存储 API 密钥

```rust
// ❌ 不要硬编码密钥
let provider = OpenAiProvider::builder()
    .api_key("sk-...")
    .build()?;

// ✅ 使用环境变量
let provider = OpenAiProvider::builder()
    .api_key(&std::env::var("OPENAI_API_KEY")?)
    .build()?;
```

### 2. 错误处理

```rust
// ✅ 优雅处理错误
match agent.run(&message, None).await {
    Ok(result) => {
        println!("{}", result.content);
        log_usage(&result.usage);
    }
    Err(ulnclaw::AgentError::Provider(msg)) => {
        eprintln!("提供商错误：{}", msg);
        // 重试或回退
    }
    Err(ulnclaw::AgentError::IterationLimit(n)) => {
        eprintln!("达到迭代限制：{}", n);
        // 简化任务或增加限制
    }
    Err(e) => {
        eprintln!("意外错误：{}", e);
        // 记录并告警
    }
}
```

### 3. 工具设计

```rust
// ✅ 清晰的描述和 Schema
tool("query_database")
    .description("在指定数据库上执行只读 SQL 查询")
    .parameters(json!({
        "type": "object",
        "properties": {
            "database": {
                "type": "string",
                "description": "数据库名称"
            },
            "query": {
                "type": "string",
                "description": "SQL SELECT 查询"
            }
        },
        "required": ["database", "query"]
    }))
    .handler(|args| async move {
        // 验证输入
        let db = args["database"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("database 是必需的"))?;
        let query = args["query"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("query 是必需的"))?;
        
        // 安全执行
        let result = execute_query(db, query).await?;
        Ok(json!({"rows": result, "count": result.len()}))
    })
    .build()?
```

### 4. 会话管理

```rust
// ✅ 持久化会话以支持多轮对话
let session_store = MemorySessionStore::new();

// 第一条消息
let result1 = agent.run("你好", None).await?;
let mut session = ulnclaw::session::new_session("user-123");
session.messages = result1.conversation;
session_store.save_session(&session)?;

// 第二条消息（带历史）
let history = session_store.load_session("user-123")?
    .map(|s| s.messages);
let result2 = agent.run("你好吗？", history).await?;
```

### 5. 监控和日志

```rust
use tracing::{info, warn, error};

let agent = Agent::new(Arc::new(provider), tools)
    .with_callbacks(AgentCallbacks {
        on_tool_start: Some(Box::new(|name, args| {
            info!(tool = name, args = %args, "工具执行开始");
        })),
        on_tool_complete: Some(Box::new(|name, result| {
            info!(tool = name, "工具执行完成");
        })),
        on_step: Some(Box::new(|iteration| {
            info!(iteration = iteration, "代理迭代完成");
        })),
        ..Default::default()
    });
```

### 6. 测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ulnclaw::prelude::*;

    #[tokio::test]
    async fn test_tool_registration() {
        let mut registry = ToolRegistry::new();
        
        registry.register(tool("test_tool")
            .description("测试工具")
            .handler(|_| async { Ok(json!({"success": true})) })
            .build()
            .unwrap());

        assert!(registry.has("test_tool"));
        assert_eq!(registry.len(), 1);
    }

    #[tokio::test]
    async fn test_tool_dispatch() {
        let mut registry = ToolRegistry::new();
        
        registry.register(tool("echo")
            .description("回显输入")
            .handler(|args| async move {
                Ok(json!({"echo": args}))
            })
            .build()
            .unwrap());

        let result = registry.dispatch("echo", json!({"msg": "hello"})).await.unwrap();
        assert_eq!(result["echo"]["msg"], "hello");
    }
}
```

## 故障排除

### 常见问题

**问题**："API 错误 (401): 无效的 API 密钥"
- **解决方案**：检查 API 密钥设置是否正确，使用环境变量

**问题**："工具未找到：xxx"
- **解决方案**：确保工具已注册且名称正确（区分大小写）

**问题**："超出迭代限制：50 次迭代"
- **解决方案**：在 `AgentConfig` 中增加 `max_iterations` 或简化任务

**问题**："提供商错误：连接超时"
- **解决方案**：检查网络连接，增加提供商配置中的超时时间

**问题**："JSON 错误：无效类型"
- **解决方案**：确保工具处理器返回有效的 JSON 值

### 调试模式

启用详细日志：

```rust
use tracing_subscriber;

tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

## 下一步

- 阅读 [工具系统指南](tools.md) 了解高级工具模式
- 阅读 [提供商系统指南](providers.md) 了解实现自定义提供商
- 查看 [API 参考](api-reference.md) 了解完整类型文档
- 参见 [开发指南](development.md) 了解贡献 ulnclaw
