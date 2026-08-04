//! MCP client — port of the core of hermes' tools/mcp_tool.py
//!
//! Model Context Protocol (stdio transport) client: spawns the server
//! process, performs `initialize`, discovers tools via `tools/list`, and
//! proxies `tools/call`. Discovered tools are registered into the tool
//! registry as `mcp__<server>__<tool>`.

use crate::error::{AgentError, Result};
use crate::tools::{tool, ToolContext, ToolRegistry};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Configuration for one MCP server (from config.toml [[mcp.servers]]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A running MCP server connection (stdio JSON-RPC).
pub struct McpClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    pending: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Result<Value>>>>>,
    next_id: Arc<Mutex<u64>>,
    reader_handle: Option<tokio::task::JoinHandle<()>>,
}

impl McpClient {
    /// Spawn the server and run the initialize handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in &config.env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AgentError::Tool(format!("spawn MCP server '{}': {}", config.name, e)))?;
        let stdin = child.stdin.take().ok_or_else(|| AgentError::Tool("MCP stdin missing".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| AgentError::Tool("MCP stdout missing".into()))?;

        let pending: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Result<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_clone = pending.clone();

        let reader_handle = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let (Some(id), Some(result)) = (
                    message.get("id").and_then(|v| v.as_u64()),
                    message.get("result").cloned(),
                ) {
                    let mut pending = pending_clone.lock().await;
                    if let Some(sender) = pending.remove(&id) {
                        sender.send(Ok(result)).ok();
                    }
                } else if let (Some(id), Some(error)) = (
                    message.get("id").and_then(|v| v.as_u64()),
                    message.get("error").cloned(),
                ) {
                    let mut pending = pending_clone.lock().await;
                    if let Some(sender) = pending.remove(&id) {
                        sender
                            .send(Err(AgentError::Tool(format!("MCP error: {}", error))))
                            .ok();
                    }
                }
            }
        });

        let mut client = Self {
            child,
            stdin,
            pending,
            next_id: Arc::new(Mutex::new(1)),
            reader_handle: Some(reader_handle),
        };

        // Initialize handshake.
        let init = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "ulnclaw", "version": crate::VERSION}
        });
        let result = client.request("initialize", init).await?;
        let _server_info = result.get("serverInfo").cloned();
        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = serde_json::to_string(&message)?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| AgentError::Tool(format!("MCP write: {}", e)))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| AgentError::Tool(format!("MCP flush: {}", e)))?;
        match tokio::time::timeout(std::time::Duration::from_secs(30), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AgentError::Tool("MCP channel closed".into())),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(AgentError::Tool(format!("MCP '{}' timed out", method)))
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let mut line = serde_json::to_string(&message)?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| AgentError::Tool(format!("MCP write: {}", e)))?;
        Ok(())
    }

    /// List the server's tools.
    pub async fn list_tools(&mut self) -> Result<Vec<Value>> {
        let result = self.request("tools/list", json!({})).await?;
        Ok(result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Call a tool on the server.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
            .await
    }

    /// Shut down the server process.
    pub async fn close(&mut self) {
        self.child.kill().await.ok();
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.child.start_kill().ok();
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
    }
}

/// Register all tools of one MCP server into the registry.
/// Tool names are prefixed `mcp__<server>__`.
pub async fn register_mcp_server(
    registry: &mut ToolRegistry,
    config: &McpServerConfig,
) -> Result<usize> {
    let mut client = McpClient::connect(config).await?;
    let tools = client.list_tools().await?;
    let client = Arc::new(Mutex::new(client));
    let mut count = 0usize;
    for tool_def in tools {
        let Some(remote_name) = tool_def.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let description = tool_def
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("(no description)")
            .to_string();
        let parameters = tool_def
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        let qualified = format!("mcp__{}__{}", config.name, remote_name);
        let remote_name = remote_name.to_string();
        let client = client.clone();
        registry.register(
            tool(qualified)
                .description(format!("[MCP {}] {}", config.name, description))
                .parameters(parameters)
                .handler(move |args, _ctx: Arc<ToolContext>| {
                    let client = client.clone();
                    let remote_name = remote_name.clone();
                    async move {
                        let mut client = client.lock().await;
                        let result = client.call_tool(&remote_name, args).await?;
                        // MCP results: {content: [{type: text, text}], isError}
                        if result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let text = result
                                .pointer("/content/0/text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown MCP error");
                            return Ok(json!({"success": false, "error": text}));
                        }
                        Ok(result)
                    }
                })
                .toolset(format!("mcp:{}", config.name))
                .emoji("🔗")
                .build()?,
        );
        count += 1;
    }
    Ok(count)
}
