# 开发指南

贡献和扩展 ulnclaw 的指南。

## 目录

- [开发环境设置](#开发环境设置)
- [项目结构](#项目结构)
- [构建和测试](#构建和测试)
- [添加新提供商](#添加新提供商)
- [添加工具](#添加工具)
- [添加新会话后端](#添加新会话后端)
- [代码风格](#代码风格)
- [测试指南](#测试指南)
- [文档](#文档)
- [贡献](#贡献)

## 开发环境设置

### 先决条件

- Rust 1.70+（2021 版本）
- Cargo
- Git

### 克隆和设置

```bash
# 克隆仓库
git clone https://gitee.com/ushaw/ulnclaw.git
cd ulnclaw

# 构建项目
cargo build

# 运行测试
cargo test

# 检查问题
cargo clippy
```

### IDE 设置

**VS Code / VSCodium：**
```bash
# 安装 rust-analyzer 扩展
code --install-extension rust-lang.rust-analyzer

# 安装 Even Better TOML
code --install-extension tamasfe.even-better-toml
```

**推荐设置** (`.vscode/settings.json`)：
```json
{
    "rust-analyzer.checkOnSave.command": "clippy",
    "rust-analyzer.cargo.features": "all",
    "editor.formatOnSave": true
}
```

## 项目结构

```
ulnclaw/
├── Cargo.toml                 # 项目清单
├── Cargo.lock                 # 依赖锁定文件
├── README.md                  # 项目文档
├── src/
│   ├── lib.rs                # 库入口点（112 行）
│   ├── error.rs              # 错误类型（74 行）
│   ├── agent/
│   │   └── mod.rs            # 核心代理循环（367 行）
│   ├── provider/
│   │   ├── mod.rs            # Provider trait 和类型（214 行）
│   │   └── openai.rs         # OpenAI 提供商（371 行）
│   ├── tools/
│   │   └── mod.rs            # 工具注册表（287 行）
│   ├── session/
│   │   └── mod.rs            # 会话管理（144 行）
│   └── context/
│       └── mod.rs            # 上下文管理（200 行）
├── tests/
│   └── integration_test.rs   # 集成测试（258 行）
└── docs/                      # 文档
    ├── en/                    # 英文文档
    └── zh/                    # 中文文档
```

### 模块职责

**lib.rs**
- 公共 API 导出
- Prelude 模块提供便捷导入
- 版本信息

**error.rs**
- `AgentError` 枚举包含所有错误类型
- 错误转换实现
- 辅助构造函数

**agent/mod.rs**
- `Agent` 结构体 - 主编排器
- 对话循环实现
- 工具分发逻辑
- 回调系统

**provider/mod.rs**
- `Provider` trait 定义
- 核心类型：`Message`, `ToolCall`, `Usage`
- `ProviderConfig` 用于动态实例化

**provider/openai.rs**
- `OpenAiProvider` 实现
- HTTP 客户端配置
- 请求/响应序列化

**tools/mod.rs**
- `ToolRegistry` - 中心注册表
- `Tool` 和 `ToolDefinition` 类型
- `ToolBuilder` 流畅 API
- 工具集管理

**session/mod.rs**
- `SessionStore` trait
- `MemorySessionStore` 实现
- 会话生命周期管理

**context/mod.rs**
- `PromptBuilder` 用于系统提示
- `ContextCompressor` 用于优化
- 令牌估计

## 构建和测试

### 构建命令

```bash
# 开发构建
cargo build

# 发布构建
cargo build --release

# 仅检查不构建
cargo check

# 为 musl 构建（静态二进制）
cargo build --release --target x86_64-unknown-linux-musl
```

### 测试命令

```bash
# 运行所有测试
cargo test

# 运行测试并显示输出
cargo test -- --nocapture

# 运行特定测试
cargo test test_tool_registry

# 并行运行测试
cargo test -- --test-threads=4

# 生成覆盖率报告
cargo install cargo-tarpaulin
cargo tarpaulin --out Html
```

### 代码检查和格式化

```bash
# 格式化代码
cargo fmt

# 检查格式
cargo fmt --check

# 运行 clippy
cargo clippy

# 运行所有特性的 clippy
cargo clippy --all-features

# 自动修复 clippy 警告
cargo clippy --fix
```

### 文档

```bash
# 生成文档
cargo doc --no-deps

# 在浏览器中打开文档
cargo doc --no-deps --open

# 检查文档链接
cargo doc --no-deps
# 然后手动检查断开的链接
```

## 添加新提供商

### 步骤 1：创建提供商模块

创建 `src/provider/your_provider.rs`：

```rust
use super::{Message, Provider, ProviderRequest, ProviderResponse, ToolCall, Usage};
use crate::error::{AgentError, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct YourProvider {
    client: Client,
    endpoint: String,
    api_key: Option<String>,
    model: String,
    name: String,
}

impl YourProvider {
    pub fn builder() -> YourProviderBuilder {
        YourProviderBuilder::default()
    }
}

pub struct YourProviderBuilder {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    name: Option<String>,
}

impl Default for YourProviderBuilder {
    fn default() -> Self {
        Self {
            endpoint: None,
            api_key: None,
            model: None,
            name: None,
        }
    }
}

impl YourProviderBuilder {
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

    pub fn build(self) -> Result<YourProvider> {
        let endpoint = self.endpoint
            .ok_or_else(|| AgentError::config("endpoint 是必需的"))?;
        let model = self.model
            .ok_or_else(|| AgentError::config("model 是必需的"))?;
        let name = self.name.unwrap_or_else(|| model.clone());

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AgentError::Internal(format!("HTTP 客户端错误：{}", e)))?;

        Ok(YourProvider {
            client,
            endpoint,
            api_key: self.api_key,
            model,
            name,
        })
    }
}

#[async_trait]
impl Provider for YourProvider {
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        // 实现 API 调用
        // 1. 将请求转换为 API 格式
        // 2. 发送 HTTP 请求
        // 3. 解析响应
        // 4. 转换为 ProviderResponse
        
        todo!("实现你的提供商")
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn name(&self) -> &str {
        &self.name
    }
}
```

### 步骤 2：导出提供商

更新 `src/provider/mod.rs`：

```rust
pub mod openai;
pub mod your_provider;  // 添加这行

pub use your_provider::YourProvider;  // 添加这行
```

### 步骤 3：添加到 ProviderConfig

更新 `src/provider/mod.rs` 中的 `ProviderConfig::build()`：

```rust
impl ProviderConfig {
    pub fn build(&self) -> Result<Box<dyn Provider>> {
        match self.kind {
            ProviderKind::OpenAiCompatible | ProviderKind::Ollama | ProviderKind::LlamaCpp => {
                // ... 现有代码
            }
            ProviderKind::YourProvider => {  // 添加这个
                let mut builder = your_provider::YourProvider::builder()
                    .endpoint(&self.endpoint)
                    .model(&self.model)
                    .name(&self.name);

                if let Some(ref key) = self.api_key {
                    builder = builder.api_key(key);
                }

                Ok(Box::new(builder.build()?))
            }
            // ...
        }
    }
}
```

### 步骤 4：添加 ProviderKind 变体

更新 `ProviderKind` 枚举：

```rust
pub enum ProviderKind {
    OpenAiCompatible,
    Ollama,
    LlamaCpp,
    Anthropic,
    Local,
    YourProvider,  // 添加这个
}
```

### 步骤 5：编写测试

创建 `tests/your_provider_test.rs`：

```rust
use ulnclaw::provider::YourProvider;

#[tokio::test]
async fn test_your_provider_builder() {
    let provider = YourProvider::builder()
        .endpoint("https://api.example.com")
        .model("test-model")
        .build();
    
    assert!(provider.is_ok());
}

#[tokio::test]
async fn test_your_provider_missing_endpoint() {
    let provider = YourProvider::builder()
        .model("test-model")
        .build();
    
    assert!(provider.is_err());
}
```

## 添加工具

### 简单工具

```rust
use ulnclaw::prelude::*;

let tool = tool("greet")
    .description("按姓名问候某人")
    .parameters(json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"}
        },
        "required": ["name"]
    }))
    .handler(|args| async move {
        let name = args["name"].as_str().unwrap_or("陌生人");
        Ok(json!({"greeting": format!("你好，{}！", name)}))
    })
    .toolset("social")
    .build()?;

registry.register(tool);
```

### 带验证的复杂工具

```rust
tool("calculate")
    .description("计算数学表达式")
    .parameters(json!({
        "type": "object",
        "properties": {
            "expression": {"type": "string"},
            "precision": {"type": "integer", "default": 2}
        },
        "required": ["expression"]
    }))
    .handler(|args| async move {
        let expr = args["expression"].as_str()
            .ok_or_else(|| ulnclaw::AgentError::tool("expression 必须是字符串"))?;
        
        let precision = args["precision"].as_u64().unwrap_or(2) as usize;
        
        // 验证表达式（白名单允许的字符）
        if !expr.chars().all(|c| c.is_digit(10) || "+-*/(). ".contains(c)) {
            return Err(ulnclaw::AgentError::tool("表达式中有无效字符"));
        }
        
        // 计算（使用安全的计算器）
        let result = evaluate_expression(expr)?;
        
        Ok(json!({
            "expression": expr,
            "result": result,
            "precision": precision
        }))
    })
    .build()?
```

### 危险工具（需要确认）

```rust
tool("delete_file")
    .description("从文件系统删除文件")
    .parameters(json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"}
        },
        "required": ["path"]
    }))
    .handler(|args| async move {
        let path = args["path"].as_str().unwrap();
        std::fs::remove_file(path)?;
        Ok(json!({"deleted": path}))
    })
    .dangerous(true)  // 标记为危险
    .build()?
```

### 带外部依赖的工具

```rust
tool("http_request")
    .description("发送 HTTP 请求")
    .parameters(json!({
        "type": "object",
        "properties": {
            "url": {"type": "string"},
            "method": {"type": "string", "enum": ["GET", "POST", "PUT", "DELETE"]}
        },
        "required": ["url"]
    }))
    .handler(|args| async move {
        let url = args["url"].as_str().unwrap();
        let method = args["method"].as_str().unwrap_or("GET");
        
        let client = reqwest::Client::new();
        let response = match method {
            "GET" => client.get(url).send().await?,
            "POST" => client.post(url).send().await?,
            // ...
            _ => return Err(ulnclaw::AgentError::tool("不支持的方法")),
        };
        
        let status = response.status().as_u16();
        let body = response.text().await?;
        
        Ok(json!({
            "status": status,
            "body": body
        }))
    })
    .build()?
```

## 添加新会话后端

### SQLite 后端示例

创建 `src/session/sqlite.rs`：

```rust
use super::{Session, SessionStore};
use crate::error::{AgentError, Result};
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
}

impl SqliteSessionStore {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| AgentError::session(format!("无法打开数据库：{}", e)))?;
        
        // 创建表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                messages TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                parent_id TEXT,
                metadata TEXT NOT NULL
            )",
            [],
        ).map_err(|e| AgentError::session(format!("无法创建表：{}", e)))?;
        
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save_session(&self, session: &Session) -> Result<()> {
        let conn = self.conn.lock()
            .map_err(|e| AgentError::session(format!("锁错误：{}", e)))?;
        
        let messages_json = serde_json::to_string(&session.messages)?;
        let metadata_json = serde_json::to_string(&session.metadata)?;
        
        conn.execute(
            "INSERT OR REPLACE INTO sessions 
             (id, conversation_id, messages, created_at_ms, updated_at_ms, parent_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.id,
                session.conversation_id,
                messages_json,
                session.created_at_ms,
                session.updated_at_ms,
                session.parent_id,
                metadata_json,
            ],
        ).map_err(|e| AgentError::session(format!("无法保存会话：{}", e)))?;
        
        Ok(())
    }

    fn load_session(&self, session_id: &str) -> Result<Option<Session>> {
        let conn = self.conn.lock()
            .map_err(|e| AgentError::session(format!("锁错误：{}", e)))?;
        
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, messages, created_at_ms, updated_at_ms, parent_id, metadata
             FROM sessions WHERE id = ?1"
        ).map_err(|e| AgentError::session(format!("查询错误：{}", e)))?;
        
        let session = stmt.query_row(params![session_id], |row| {
            let messages_json: String = row.get(2)?;
            let metadata_json: String = row.get(6)?;
            
            Ok((
                row.get(0)?,
                row.get(1)?,
                messages_json,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                metadata_json,
            ))
        }).ok();
        
        if let Some((id, conv_id, messages_json, created, updated, parent, metadata_json)) = session {
            let messages = serde_json::from_str(&messages_json)?;
            let metadata = serde_json::from_str(&metadata_json)?;
            
            Ok(Some(Session {
                id,
                conversation_id: conv_id,
                messages,
                created_at_ms: created,
                updated_at_ms: updated,
                parent_id: parent,
                metadata,
            }))
        } else {
            Ok(None)
        }
    }

    // 实现其他方法...
    fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        todo!()
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        todo!()
    }

    fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<Session>> {
        todo!()
    }
}
```

## 代码风格

### 格式化

所有代码必须用 `cargo fmt` 格式化：

```bash
cargo fmt
```

### 命名约定

- **类型**：`PascalCase`（例如，`AgentConfig`, `ToolRegistry`）
- **函数/方法**：`snake_case`（例如，`chat_completion`, `load_session`）
- **常量**：`SCREAMING_SNAKE_CASE`（例如，`MAX_ITERATIONS`）
- **模块**：`snake_case`（例如，`provider`, `tools`）

### 文档

所有公共项必须有文档注释：

```rust
/// 函数的简短描述。
///
/// 如果需要更详细的解释。
///
/// # 参数
///
/// * `arg1` - arg1 的描述
/// * `arg2` - arg2 的描述
///
/// # 返回
///
/// 返回值的描述
///
/// # 示例
///
/// ```rust
/// let result = my_function(42);
/// assert_eq!(result, 84);
/// ```
pub fn my_function(arg1: i32, arg2: i32) -> i32 {
    arg1 + arg2
}
```

### 错误处理

使用 `?` 运算符进行错误传播：

```rust
// ✅ 好的
fn read_file(path: &str) -> Result<String> {
    let content = std::fs::read_to_string(path)?;
    Ok(content)
}

// ❌ 不好的
fn read_file(path: &str) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(e) => Err(AgentError::Internal(e.to_string())),
    }
}
```

### Async/Await

优先使用 async/await 而不是回调：

```rust
// ✅ 好的
async fn fetch_data() -> Result<String> {
    let response = client.get(url).send().await?;
    let body = response.text().await?;
    Ok(body)
}

// ❌ 不好的（基于回调）
fn fetch_data(callback: impl FnOnce(String)) {
    // ...
}
```

## 测试指南

### 单元测试

将单元测试放在与代码相同的文件中：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_addition() {
        assert_eq!(add(2, 3), 5);
    }

    #[test]
    fn test_negative_numbers() {
        assert_eq!(add(-1, -1), -2);
    }
}
```

### 集成测试

将集成测试放在 `tests/` 目录中：

```rust
// tests/integration_test.rs
use ulnclaw::prelude::*;

#[tokio::test]
async fn test_full_conversation() {
    let provider = create_mock_provider();
    let tools = create_test_tools();
    let agent = Agent::new(Arc::new(provider), tools);
    
    let result = agent.run("你好", None).await.unwrap();
    assert!(!result.content.is_empty());
}
```

### Mock 提供商

创建用于测试的 mock 提供商：

```rust
use async_trait::async_trait;

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
```

## 文档

### 编写文档

所有文档应该：
- 清晰简洁
- 包含代码示例
- 涵盖常见用例
- 提及陷阱和边缘情况

### 文档结构

```
docs/
├── en/
│   ├── architecture.md    # 系统设计
│   ├── api-reference.md   # 完整 API 文档
│   ├── integration.md     # 集成指南
│   ├── development.md     # 本文件
│   ├── tools.md          # 工具系统指南
│   └── providers.md      # 提供商系统指南
└── zh/
    └── （中文翻译）
```

### 构建文档

```bash
# 生成 HTML 文档
cargo doc --no-deps --open

# 检查断开的链接
cargo doc --no-deps
```

## 贡献

### 工作流程

1. **Fork 仓库**
2. **创建特性分支**
   ```bash
   git checkout -b feature/my-feature
   ```
3. **进行更改**
   - 编写代码
   - 添加测试
   - 更新文档
4. **运行检查**
   ```bash
   cargo fmt
   cargo clippy
   cargo test
   ```
5. **提交更改**
   ```bash
   git commit -m "feat: 添加我的特性"
   ```
6. **推送并创建 PR**
   ```bash
   git push origin feature/my-feature
   ```

### 提交消息

遵循约定式提交：

- `feat:` - 新特性
- `fix:` - Bug 修复
- `docs:` - 文档更改
- `style:` - 代码风格更改
- `refactor:` - 代码重构
- `test:` - 添加测试
- `chore:` - 维护任务

**示例：**
```
feat: 添加 Anthropic 提供商支持
fix: 解决工具分发竞态条件
docs: 更新集成指南并添加示例
refactor: 简化上下文压缩逻辑
test: 为会话持久化添加单元测试
```

### 代码审查清单

- [ ] 代码已用 `cargo fmt` 格式化
- [ ] 没有 clippy 警告
- [ ] 所有测试通过
- [ ] 新特性有测试
- [ ] 文档已更新
- [ ] 提交消息遵循约定
- [ ] 没有破坏性更改（或在 CHANGELOG 中记录）

### 报告问题

报告问题时，包括：

- **ulnclaw 版本**
- **Rust 版本**
- **操作系统**
- **最小复现代码**
- **期望行为**
- **实际行为**
- **错误消息/日志**

### 特性请求

对于特性请求，描述：

- **用例** - 这解决了什么问题？
- **提议的解决方案** - 它应该如何工作？
- **考虑的替代方案** - 尝试了哪些其他方法？
- **附加上下文** - 任何相关信息

## 路线图

### 计划特性

- [ ] 流式响应
- [ ] 上下文压缩
- [ ] 子代理委托
- [ ] MCP 协议支持
- [ ] SQLite 会话后端
- [ ] Anthropic 原生提供商
- [ ] 自动重试逻辑
- [ ] 指标和追踪
- [ ] 插件系统
- [ ] 多模态支持

### 贡献想法

- 实现路线图中缺少的特性
- 添加更多提供商（Google Gemini、Cohere 等）
- 改进文档
- 添加示例
- 撰写博客文章
- 创建视频教程

## 获取帮助

- **文档**：阅读 `docs/` 目录中的文档
- **问题**：在 Gitee 上搜索现有问题
- **讨论**：在 Gitee 上开启讨论
- **邮件**：联系维护者

## 许可证

通过贡献，你同意你的贡献将在 MIT 或 Apache-2.0 许可证下许可。
