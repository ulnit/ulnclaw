# 提供商系统指南

ulnclaw 提供商抽象和实现的完整指南。

## 目录

- [概述](#概述)
- [Provider Trait](#provider-trait)
- [OpenAI 提供商](#openai-提供商)
- [支持的提供商](#支持的提供商)
- [实现自定义提供商](#实现自定义提供商)
- [提供商配置](#提供商配置)
- [错误处理](#错误处理)
- [最佳实践](#最佳实践)

## 概述

ulnclaw 的提供商系统在通用 trait 后面抽象了 AI 模型后端，允许在不同提供商之间无缝切换。

**关键特性：**
- 统一的 `Provider` trait 用于所有后端
- OpenAI 兼容实现（涵盖大多数提供商）
- 构建器模式用于配置
- Async/await 用于非阻塞 I/O
- 完整的错误处理

## Provider Trait

AI 模型提供商的核心抽象：

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    /// 发送聊天完成请求
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse>;
    
    /// 获取模型名称
    fn model(&self) -> &str;
    
    /// 获取提供商名称
    fn name(&self) -> &str;
    
    /// 检查提供商是否可用
    async fn is_available(&self) -> bool {
        true
    }
}
```

### 请求/响应类型

```rust
pub struct ProviderRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub stop: Option<Vec<String>>,
}

pub struct ProviderResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub model: String,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
}
```

## OpenAI 提供商

`OpenAiProvider` 实现了 OpenAI Chat Completions API 格式，该格式被其他提供商广泛采用。

### 基础用法

```rust
use ulnclaw::prelude::*;

let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key("sk-...")
    .model("gpt-4")
    .build()?;

let agent = Agent::new(Arc::new(provider), tools);
```

### 构建器模式

```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")  // 必需
    .api_key("sk-...")                       // 可选（某些提供商不需要）
    .model("gpt-4")                          // 必需
    .name("我的提供商")                        // 可选（默认为模型名称）
    .max_tokens(4096)                        // 可选
    .temperature(0.7)                        // 可选
    .build()?;
```

### API URL 解析

提供商自动解析 API URL：

```rust
// 这些都解析到相同的端点：
"https://api.openai.com/v1"
"https://api.openai.com/v1/"
"https://api.openai.com"
// → "https://api.openai.com/v1/chat/completions"

// 直接端点：
"https://api.openai.com/v1/chat/completions"
// → "https://api.openai.com/v1/chat/completions"
```

### 请求流程

```
1. 将 Message/ToolCall 转换为 OpenAI API 格式
   ↓
2. 构建带 headers 的 HTTP 请求
   ├─ Content-Type: application/json
   ├─ Authorization: Bearer <api_key>
   └─ Body: JSON 负载
   ↓
3. 发送 POST 到 /v1/chat/completions
   ↓
4. 解析响应
   ├─ 提取内容
   ├─ 提取 tool_calls
   ├─ 提取使用统计
   └─ 提取推理（如果存在）
   ↓
5. 转换为 ProviderResponse
```

## 支持的提供商

### OpenAI

```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key(&std::env::var("OPENAI_API_KEY")?)
    .model("gpt-4")
    .build()?;
```

**模型：** gpt-4, gpt-4-turbo, gpt-3.5-turbo

### Anthropic (Claude)

使用 OpenAI 兼容格式：

```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://api.anthropic.com/v1")
    .api_key(&std::env::var("ANTHROPIC_API_KEY")?)
    .model("claude-3-opus-20240229")
    .build()?;
```

**模型：** claude-3-opus, claude-3-sonnet, claude-3-haiku

### DashScope (阿里巴巴 Qwen)

```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://dashscope.aliyuncs.com/compatible-mode")
    .api_key(&std::env::var("DASHSCOPE_API_KEY")?)
    .model("qwen-plus")
    .build()?;
```

**模型：** qwen-turbo, qwen-plus, qwen-max

### OpenRouter

```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://openrouter.ai/api/v1")
    .api_key(&std::env::var("OPENROUTER_API_KEY")?)
    .model("anthropic/claude-3-opus")
    .build()?;
```

**模型：** OpenRouter 上可用的任何模型

### Ollama (本地)

```rust
let provider = OpenAiProvider::builder()
    .endpoint("http://localhost:11434/v1")
    .model("llama2")
    .build()?;
```

**模型：** 用 `ollama pull` 拉取的任何模型

### llama.cpp 服务器

```rust
let provider = OpenAiProvider::builder()
    .endpoint("http://localhost:8080/v1")
    .model("local-model")
    .build()?;
```

**注意：** llama.cpp 服务器忽略模型名称

## 实现自定义提供商

### 步骤 1：创建提供商结构体

```rust
use ulnclaw::provider::{Provider, ProviderRequest, ProviderResponse};
use ulnclaw::error::Result;
use async_trait::async_trait;

pub struct CustomProvider {
    client: reqwest::Client,
    endpoint: String,
    api_key: Option<String>,
    model: String,
    name: String,
}
```

### 步骤 2：实现构建器

```rust
pub struct CustomProviderBuilder {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    name: Option<String>,
}

impl CustomProvider {
    pub fn builder() -> CustomProviderBuilder {
        CustomProviderBuilder {
            endpoint: None,
            api_key: None,
            model: None,
            name: None,
        }
    }
}

impl CustomProviderBuilder {
    pub fn endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = Some(endpoint.to_string());
        self
    }

    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = Some(key.to_string());
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn build(self) -> Result<CustomProvider> {
        let endpoint = self.endpoint
            .ok_or_else(|| ulnclaw::AgentError::config("endpoint 是必需的"))?;
        let model = self.model
            .ok_or_else(|| ulnclaw::AgentError::config("model 是必需的"))?;
        let name = self.name.unwrap_or_else(|| model.clone());

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| ulnclaw::AgentError::Internal(e.to_string()))?;

        Ok(CustomProvider {
            client,
            endpoint,
            api_key: self.api_key,
            model,
            name,
        })
    }
}
```

### 步骤 3：实现 Provider Trait

```rust
#[async_trait]
impl Provider for CustomProvider {
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        // 1. 将请求转换为 API 格式
        let api_request = self.convert_request(&request);
        
        // 2. 构建 HTTP 请求
        let url = format!("{}/chat/completions", self.endpoint);
        let mut req_builder = self.client.post(&url)
            .json(&api_request);
        
        if let Some(ref key) = self.api_key {
            req_builder = req_builder.bearer_auth(key);
        }
        
        // 3. 发送请求
        let response = req_builder.send().await
            .map_err(|e| ulnclaw::AgentError::Provider(e.to_string()))?;
        
        // 4. 检查状态
        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(ulnclaw::AgentError::Provider(error));
        }
        
        // 5. 解析响应
        let api_response: ApiResponse = response.json().await
            .map_err(|e| ulnclaw::AgentError::Provider(e.to_string()))?;
        
        // 6. 转换为 ProviderResponse
        self.convert_response(api_response)
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn name(&self) -> &str {
        &self.name
    }
}
```

### 步骤 4：添加转换方法

```rust
impl CustomProvider {
    fn convert_request(&self, request: &ProviderRequest) -> serde_json::Value {
        // 将 Message 和 ToolCall 转换为 API 格式
        serde_json::json!({
            "model": request.model,
            "messages": request.messages,
            "tools": request.tools,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
        })
    }

    fn convert_response(&self, response: ApiResponse) -> Result<ProviderResponse> {
        // 将 API 响应解析为 ProviderResponse
        Ok(ProviderResponse {
            content: response.choices[0].message.content.clone(),
            tool_calls: vec![],
            usage: None,
            model: self.model.clone(),
            reasoning: None,
            finish_reason: None,
        })
    }
}
```

### 步骤 5：导出和使用

```rust
// 在你的 lib.rs 或 mod.rs 中
pub mod custom_provider;
pub use custom_provider::CustomProvider;

// 用法
let provider = CustomProvider::builder()
    .endpoint("https://api.custom.com")
    .model("custom-model")
    .build()?;

let agent = Agent::new(Arc::new(provider), tools);
```

## 提供商配置

### 动态配置

使用 `ProviderConfig` 进行运行时的提供商实例化：

```rust
use ulnclaw::provider::{ProviderConfig, ProviderKind};

let config = ProviderConfig {
    name: "我的提供商".to_string(),
    kind: ProviderKind::OpenAiCompatible,
    endpoint: "https://api.openai.com/v1".to_string(),
    api_key: Some("sk-...".to_string()),
    model: "gpt-4".to_string(),
    max_tokens: Some(4096),
    temperature: Some(0.7),
    fallback_providers: vec![],
};

let provider = config.build()?;
```

### 提供商类型

```rust
pub enum ProviderKind {
    OpenAiCompatible,  // OpenAI, Anthropic, DashScope 等
    Ollama,            // 本地 Ollama 服务器
    LlamaCpp,          // 本地 llama.cpp 服务器
    Anthropic,         // 原生 Anthropic API（未来）
    Local,             // 嵌入式模型（未来）
}
```

### 回退提供商

配置回退提供商以进行自动故障转移：

```rust
let config = ProviderConfig {
    name: "主要".to_string(),
    kind: ProviderKind::OpenAiCompatible,
    endpoint: "https://api.openai.com/v1".to_string(),
    api_key: Some("sk-primary".to_string()),
    model: "gpt-4".to_string(),
    fallback_providers: vec![
        "回退 1".to_string(),
        "回退 2".to_string(),
    ],
    ..Default::default()
};
```

在你的应用中实现回退逻辑：

```rust
async fn try_providers(configs: Vec<ProviderConfig>, message: &str) -> Result<String> {
    let mut last_error = None;
    
    for config in configs {
        match config.build() {
            Ok(provider) => {
                let agent = Agent::new(Arc::from(provider), tools.clone());
                match agent.chat(message).await {
                    Ok(response) => return Ok(response),
                    Err(e) => last_error = Some(e),
                }
            }
            Err(e) => last_error = Some(e),
        }
    }
    
    Err(last_error.unwrap())
}
```

## 错误处理

### 提供商错误

```rust
pub enum AgentError {
    Provider(String),  // API 失败、认证错误、速率限制
    Http(reqwest::Error),  // 网络错误
    Json(serde_json::Error),  // 序列化错误
    // ...
}
```

### 常见错误场景

**认证错误：**
```rust
// 错误：Provider error: API error (401): Invalid API key
// 解决方案：检查 API 密钥设置是否正确
```

**速率限制错误：**
```rust
// 错误：Provider error: API error (429): Rate limit exceeded
// 解决方案：实现带指数退避的重试逻辑
```

**网络错误：**
```rust
// 错误：HTTP error: Connection timeout
// 解决方案：检查网络连接，增加超时时间
```

**模型未找到：**
```rust
// 错误：Provider error: API error (404): Model not found
// 解决方案：检查模型名称是否正确
```

### 错误恢复

```rust
match agent.chat(&message).await {
    Ok(response) => println!("{}", response),
    Err(ulnclaw::AgentError::Provider(msg)) => {
        if msg.contains("401") {
            eprintln!("认证失败。检查 API 密钥。");
        } else if msg.contains("429") {
            eprintln!("速率限制。重试中...");
            tokio::time::sleep(Duration::from_secs(5)).await;
            // 重试
        } else {
            eprintln!("提供商错误：{}", msg);
        }
    }
    Err(e) => eprintln!("错误：{}", e),
}
```

## 最佳实践

### 1. 使用环境变量存储 API 密钥

```rust
// ❌ 不要硬编码
let provider = OpenAiProvider::builder()
    .api_key("sk-...")
    .build()?;

// ✅ 使用环境变量
let provider = OpenAiProvider::builder()
    .api_key(&std::env::var("OPENAI_API_KEY")?)
    .build()?;
```

### 2. 设置合理的超时

```rust
let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key(&api_key)
    .model("gpt-4")
    // 超时在构建器中设置（默认：120 秒）
    .build()?;
```

### 3. 实现重试逻辑

```rust
async fn chat_with_retry(agent: &Agent, message: &str, max_retries: u32) -> Result<String> {
    let mut attempt = 0;
    
    loop {
        match agent.chat(message).await {
            Ok(response) => return Ok(response),
            Err(ulnclaw::AgentError::Provider(msg)) if msg.contains("429") => {
                attempt += 1;
                if attempt > max_retries {
                    return Err(ulnclaw::AgentError::Provider("超出最大重试次数".into()));
                }
                
                let delay = Duration::from_secs(2u64.pow(attempt));
                eprintln!("速率限制。在 {:?} 后重试...", delay);
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(e),
        }
    }
}
```

### 4. 监控使用量

```rust
let result = agent.run(&message, None).await?;

if let Some(usage) = &result.usage {
    println!("使用令牌数：{}", usage.total_tokens);
    println!("提示令牌：{}", usage.prompt_tokens);
    println!("完成令牌：{}", usage.completion_tokens);
    
    // 记录到监控系统
    log_usage(usage.total_tokens, &result.model);
}
```

### 5. 使用 Mock 提供商进行测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        async fn chat_completion(&self, _request: ProviderRequest) -> Result<ProviderResponse> {
            Ok(ProviderResponse {
                content: Some("Mock 响应".to_string()),
                tool_calls: vec![],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                }),
                model: "mock".to_string(),
                reasoning: None,
                finish_reason: Some("stop".to_string()),
            })
        }

        fn model(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "MockProvider"
        }
    }

    #[tokio::test]
    async fn test_agent_with_mock() {
        let provider = MockProvider;
        let tools = ToolRegistry::new();
        let agent = Agent::new(Arc::new(provider), tools);
        
        let response = agent.chat("你好").await.unwrap();
        assert_eq!(response, "Mock 响应");
    }
}
```

### 6. 处理流式（未来）

```rust
// 流式支持计划在未来版本中实现
// 目前使用非流式模式

let provider = OpenAiProvider::builder()
    .endpoint("https://api.openai.com/v1")
    .api_key(&api_key)
    .model("gpt-4")
    .build()?;

// stream: false（默认）
let request = ProviderRequest {
    messages: vec![],
    tools: vec![],
    model: "gpt-4".to_string(),
    stream: false,  // 非流式
    // ...
};
```

## 故障排除

### 常见问题

**问题**："API 错误 (401): 无效的 API 密钥"
- **解决方案**：验证 API 密钥设置正确，检查环境变量

**问题**："API 错误 (404): 模型未找到"
- **解决方案**：检查模型名称拼写，验证模型是否可用

**问题**："连接超时"
- **解决方案**：检查网络连接，验证端点 URL

**问题**："API 错误 (429): 超出速率限制"
- **解决方案**：实现重试逻辑，降低请求频率

**问题**："解析响应失败"
- **解决方案**：检查 API 兼容性，验证端点返回 OpenAI 格式

### 调试模式

启用详细日志：

```rust
use tracing_subscriber;

tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();

// 现在你将看到详细的 HTTP 请求/响应日志
```

## 下一步

- 阅读 [工具系统指南](tools.md) 了解实现工具
- 查看 [API 参考](api-reference.md) 了解完整类型文档
- 参见 [集成指南](integration.md) 了解在应用中使用提供商
- 查看 [开发指南](development.md) 了解贡献新提供商
