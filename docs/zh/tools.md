# 工具系统指南

ulnclaw 工具注册系统的完整指南。

## 目录

- [概述](#概述)
- [工具架构](#工具架构)
- [创建工具](#创建工具)
- [工具注册表](#工具注册表)
- [工具集](#工具集)
- [高级模式](#高级模式)
- [最佳实践](#最佳实践)
- [示例](#示例)

## 概述

ulnclaw 的工具系统受 Hermes Agent 的注册表模式启发。它提供：

- **自注册工具**：工具在模块加载时自行注册
- **JSON Schema 验证**：工具使用 JSON Schema 定义参数
- **异步处理器**：工具处理器是异步函数
- **工具集管理**：将相关工具分组到工具集
- **动态分发**：代理按名称分发工具调用

## 工具架构

### 核心类型

```rust
// 暴露给模型的工具定义
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,  // JSON Schema
}

// 带处理器的已注册工具
pub struct Tool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub toolset: String,
    pub dangerous: bool,
}

// 异步处理器类型
pub type ToolHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// 工具执行结果
pub struct ToolResult {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}
```

### 流程

```
1. 模型生成工具调用
   ↓
2. 代理提取工具名称和参数
   ↓
3. ToolRegistry.dispatch(name, args)
   ├─ 按名称查找工具
   ├─ 检查工具集是否启用
   ├─ 用参数调用处理器
   └─ 返回结果
   ↓
4. 代理将结果添加到对话
   ↓
5. 模型处理结果并继续
```

## 创建工具

### 基础工具

```rust
use ulnclaw::prelude::*;

let tool = tool("get_time")
    .description("获取当前时间")
    .parameters(json!({
        "type": "object",
        "properties": {}
    }))
    .handler(|_args| async {
        let now = chrono::Utc::now();
        Ok(json!({"time": now.to_rfc3339()}))
    })
    .build()?;

registry.register(tool);
```

### 带参数的工具

```rust
tool("calculate")
    .description("执行算术运算")
    .parameters(json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "enum": ["add", "subtract", "multiply", "divide"]
            },
            "a": {"type": "number"},
            "b": {"type": "number"}
        },
        "required": ["operation", "a", "b"]
    }))
    .handler(|args| async move {
        let op = args["operation"].as_str().unwrap();
        let a = args["a"].as_f64().unwrap();
        let b = args["b"].as_f64().unwrap();
        
        let result = match op {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" => {
                if b == 0.0 {
                    return Err(ulnclaw::AgentError::tool("除以零"));
                }
                a / b
            }
            _ => return Err(ulnclaw::AgentError::tool("未知操作")),
        };
        
        Ok(json!({"result": result}))
    })
    .build()?
```

### 带验证的工具

```rust
tool("send_email")
    .description("发送电子邮件消息")
    .parameters(json!({
        "type": "object",
        "properties": {
            "to": {"type": "string", "format": "email"},
            "subject": {"type": "string", "maxLength": 100},
            "body": {"type": "string"}
        },
        "required": ["to", "subject", "body"]
    }))
    .handler(|args| async move {
        let to = args["to"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("to 是必需的"))?;
        let subject = args["subject"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("subject 是必需的"))?;
        let body = args["body"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("body 是必需的"))?;
        
        // 验证邮箱格式
        if !to.contains('@') {
            return Err(ulnclaw::AgentError::tool("无效的邮箱格式"));
        }
        
        // 验证主题长度
        if subject.len() > 100 {
            return Err(ulnclaw::AgentError::tool("主题太长"));
        }
        
        // 发送邮件（实现）
        send_email_impl(to, subject, body).await?;
        
        Ok(json!({
            "success": true,
            "message_id": "msg-123"
        }))
    })
    .build()?
```

### 危险工具

标记需要用户确认的工具：

```rust
tool("delete_database")
    .description("删除整个数据库")
    .parameters(json!({
        "type": "object",
        "properties": {
            "database": {"type": "string"}
        },
        "required": ["database"]
    }))
    .handler(|args| async move {
        let db = args["database"].as_str().unwrap();
        // 删除数据库
        Ok(json!({"deleted": db}))
    })
    .dangerous(true)  // 标记为危险
    .build()?
```

代理可以检查 `tool.dangerous` 并在执行前提示确认。

## 工具注册表

### 创建注册表

```rust
let mut registry = ToolRegistry::new();
```

### 注册工具

```rust
registry.register(tool);
```

### 分发工具

```rust
let result = registry.dispatch("get_time", json!({})).await?;
```

### 查询工具

```rust
// 检查工具是否存在
if registry.has("get_time") {
    println!("工具已注册");
}

// 获取工具数量
println!("{} 个工具已注册", registry.len());

// 获取所有工具名称
let names = registry.names();

// 获取工具定义（用于发送给模型）
let definitions = registry.definitions();
```

### 注销工具

```rust
if let Some(tool) = registry.unregister("old_tool") {
    println!("已注销：{}", tool.definition.name);
}
```

## 工具集

工具集将相关工具分组以便管理。

### 定义工具集

```rust
tool("query_database")
    .description("查询数据库")
    .handler(|args| async { /* ... */ })
    .toolset("database")  // 分配到工具集
    .build()?

tool("list_tables")
    .description("列出数据库表")
    .handler(|args| async { /* ... */ })
    .toolset("database")  // 相同工具集
    .build()?
```

### 管理工具集

```rust
// 获取所有工具集名称
let toolsets = registry.toolset_names();

// 获取工具集中的工具
let db_tools = registry.toolset_tools("database");

// 禁用工具集
registry.disable_toolset("database");

// 启用工具集
registry.enable_toolset("database");
```

### 基于工具集的过滤

```rust
// 只发送启用的工具给模型
let definitions = registry.definitions();  // 排除禁用的工具集

// 检查工具集是否可用
if !registry.toolset_names().contains(&"database".to_string()) {
    println!("数据库工具不可用");
}
```

## 高级模式

### 带状态的工具

使用闭包捕获状态：

```rust
let counter = Arc::new(Mutex::new(0));
let counter_clone = counter.clone();

let tool = tool("increment")
    .description("递增计数器")
    .handler(move |_args| {
        let counter = counter_clone.clone();
        async move {
            let mut count = counter.lock().unwrap();
            *count += 1;
            Ok(json!({"count": *count}))
        }
    })
    .build()?;
```

### 带外部依赖的工具

```rust
use reqwest::Client;

let client = Arc::new(Client::new());
let client_clone = client.clone();

tool("fetch_url")
    .description("从 URL 获取内容")
    .parameters(json!({
        "type": "object",
        "properties": {
            "url": {"type": "string"}
        },
        "required": ["url"]
    }))
    .handler(move |args| {
        let client = client_clone.clone();
        async move {
            let url = args["url"].as_str().unwrap();
            let response = client.get(url).send().await?;
            let body = response.text().await?;
            Ok(json!({"body": body}))
        }
    })
    .build()?
```

### 工具组合

从低级工具创建高级工具：

```rust
// 低级工具
let registry = Arc::new(Mutex::new(ToolRegistry::new()));
registry.lock().unwrap().register(
    tool("http_get")
        .handler(|args| async { /* ... */ })
        .build()?
);

// 使用低级工具的高级工具
let registry_clone = registry.clone();
let tool = tool("check_api_status")
    .description("检查 API 是否健康")
    .handler(move |args| {
        let registry = registry_clone.clone();
        async move {
            let result = registry.lock().unwrap()
                .dispatch("http_get", json!({"url": "https://api.example.com/health"}))
                .await?;
            
            let status = result["status"].as_u64().unwrap_or(0);
            Ok(json!({"healthy": status == 200}))
        }
    })
    .build()?;
```

### 动态工具注册

基于配置注册工具：

```rust
fn register_tools_from_config(registry: &mut ToolRegistry, config: &Config) -> Result<()> {
    for tool_config in &config.tools {
        let tool = create_tool_from_config(tool_config)?;
        registry.register(tool);
    }
    Ok(())
}
```

## 最佳实践

### 1. 清晰的描述

```rust
// ❌ 不好
tool("calc")
    .description("做数学")
    .build()?

// ✅ 好
tool("calculate")
    .description("对两个数字执行算术运算（加、减、乘、除）")
    .build()?
```

### 2. 详细的参数 Schema

```rust
// ❌ 不好
tool("query")
    .parameters(json!({
        "type": "object",
        "properties": {
            "q": {"type": "string"}
        }
    }))
    .build()?

// ✅ 好
tool("search_database")
    .parameters(json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "搜索查询字符串"
            },
            "limit": {
                "type": "integer",
                "description": "最大结果数",
                "default": 10,
                "minimum": 1,
                "maximum": 100
            },
            "sort_by": {
                "type": "string",
                "description": "排序字段",
                "enum": ["relevance", "date", "name"]
            }
        },
        "required": ["query"]
    }))
    .build()?
```

### 3. 输入验证

```rust
tool("create_user")
    .handler(|args| async move {
        let username = args["username"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("username 是必需的"))?;
        
        // 验证用户名
        if username.len() < 3 {
            return Err(ulnclaw::AgentError::tool("用户名必须至少 3 个字符"));
        }
        
        if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(ulnclaw::AgentError::tool("用户名必须是字母数字"));
        }
        
        // 创建用户
        Ok(json!({"user_id": "user-123"}))
    })
    .build()?
```

### 4. 错误处理

```rust
tool("read_file")
    .handler(|args| async move {
        let path = args["path"].as_str().unwrap();
        
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(json!({"content": content})),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(ulnclaw::AgentError::tool(format!("文件未找到：{}", path)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(ulnclaw::AgentError::tool(format!("权限被拒绝：{}", path)))
            }
            Err(e) => {
                Err(ulnclaw::AgentError::tool(format!("读取文件失败：{}", e)))
            }
        }
    })
    .build()?
```

### 5. 结构化响应

```rust
tool("list_files")
    .handler(|args| async move {
        let path = args["path"].as_str().unwrap();
        let entries = std::fs::read_dir(path)?;
        
        let files: Vec<Value> = entries
            .filter_map(|e| e.ok())
            .map(|e| {
                json!({
                    "name": e.file_name().to_string_lossy(),
                    "is_dir": e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                    "size": e.metadata().map(|m| m.len()).unwrap_or(0)
                })
            })
            .collect();
        
        Ok(json!({
            "path": path,
            "count": files.len(),
            "files": files
        }))
    })
    .build()?
```

### 6. 幂等操作

```rust
tool("set_config")
    .description("设置配置值（幂等）")
    .handler(|args| async move {
        let key = args["key"].as_str().unwrap();
        let value = args["value"].as_str().unwrap();
        
        // 多次设置相同的值具有相同的效果
        set_config_impl(key, value).await?;
        
        Ok(json!({"key": key, "value": value, "updated": true}))
    })
    .build()?
```

## 示例

### 完整工具集

```rust
use ulnclaw::prelude::*;

fn create_filesystem_tools() -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    
    // 读取文件
    registry.register(tool("read_file")
        .description("读取文件内容")
        .parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }))
        .handler(|args| async move {
            let path = args["path"].as_str().unwrap();
            let content = std::fs::read_to_string(path)?;
            Ok(json!({"content": content, "size": content.len()}))
        })
        .toolset("filesystem")
        .build()?);
    
    // 写入文件
    registry.register(tool("write_file")
        .description("将内容写入文件")
        .parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        }))
        .handler(|args| async move {
            let path = args["path"].as_str().unwrap();
            let content = args["content"].as_str().unwrap();
            std::fs::write(path, content)?;
            Ok(json!({"written": path, "bytes": content.len()}))
        })
        .toolset("filesystem")
        .dangerous(true)
        .build()?);
    
    // 列出目录
    registry.register(tool("list_directory")
        .description("列出目录中的文件")
        .parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }))
        .handler(|args| async move {
            let path = args["path"].as_str().unwrap();
            let entries: Vec<String> = std::fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            Ok(json!({"entries": entries, "count": entries.len()}))
        })
        .toolset("filesystem")
        .build()?);
    
    Ok(registry)
}
```

### 带缓存的工具

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Cache {
    data: HashMap<String, (Value, Instant)>,
    ttl: Duration,
}

impl Cache {
    fn new(ttl: Duration) -> Self {
        Self {
            data: HashMap::new(),
            ttl,
        }
    }
    
    fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key).and_then(|(value, time)| {
            if time.elapsed() < self.ttl {
                Some(value)
            } else {
                None
            }
        })
    }
    
    fn set(&mut self, key: String, value: Value) {
        self.data.insert(key, (value, Instant::now()));
    }
}

let cache = Arc::new(Mutex::new(Cache::new(Duration::from_secs(60))));
let cache_clone = cache.clone();

tool("get_weather")
    .description("获取天气（缓存 60 秒）")
    .handler(move |args| {
        let cache = cache_clone.clone();
        async move {
            let city = args["city"].as_str().unwrap();
            
            // 检查缓存
            if let Some(cached) = cache.lock().unwrap().get(city) {
                return Ok(cached.clone());
            }
            
            // 从 API 获取
            let weather = fetch_weather(city).await?;
            
            // 更新缓存
            cache.lock().unwrap().set(city.to_string(), weather.clone());
            
            Ok(weather)
        }
    })
    .build()?
```

### 带速率限制的工具

```rust
use std::time::{Duration, Instant};

struct RateLimiter {
    last_call: Option<Instant>,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(min_interval: Duration) -> Self {
        Self {
            last_call: None,
            min_interval,
        }
    }
    
    fn check(&mut self) -> Result<()> {
        if let Some(last) = self.last_call {
            let elapsed = last.elapsed();
            if elapsed < self.min_interval {
                return Err(ulnclaw::AgentError::tool(format!(
                    "速率限制。等待 {}ms",
                    (self.min_interval - elapsed).as_millis()
                )));
            }
        }
        self.last_call = Some(Instant::now());
        Ok(())
    }
}

let limiter = Arc::new(Mutex::new(RateLimiter::new(Duration::from_secs(1))));
let limiter_clone = limiter.clone();

tool("expensive_api")
    .description("调用昂贵的 API（速率限制）")
    .handler(move |args| {
        let limiter = limiter_clone.clone();
        async move {
            limiter.lock().unwrap().check()?;
            
            // 调用 API
            let result = call_expensive_api(args).await?;
            Ok(result)
        }
    })
    .build()?
```

## 下一步

- 阅读 [提供商系统指南](providers.md) 了解实现提供商
- 查看 [API 参考](api-reference.md) 了解完整类型文档
- 参见 [集成指南](integration.md) 了解在应用中使用工具
