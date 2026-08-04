//! Tool registry - self-registering tools with schema, handler, and toolset membership
//!
//! Inspired by Hermes Agent's tools/registry.py which uses a central registry
//! where each tool file calls registry.register() at module level.

use crate::error::{AgentError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::debug;

/// Definition of a tool (schema exposed to the model)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool with its definition and async handler
pub struct Tool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    /// Toolset this tool belongs to (for grouping/enable/disable)
    pub toolset: String,
    /// Whether this tool requires user confirmation before execution
    pub dangerous: bool,
}

/// Type alias for async tool handler function
pub type ToolHandler = Arc<
    dyn Fn(
            serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send>>
        + Send
        + Sync,
>;

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            serde_json::json!({"success": false, "error": "Failed to serialize result"})
        })
    }
}

/// Central registry for all tools
pub struct ToolRegistry {
    tools: HashMap<String, Tool>,
    /// Toolset name -> list of tool names
    toolsets: HashMap<String, Vec<String>>,
    /// Disabled toolsets
    disabled_toolsets: Vec<String>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            toolsets: HashMap::new(),
            disabled_toolsets: Vec::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Tool) {
        let name = tool.definition.name.clone();
        let toolset = tool.toolset.clone();

        self.toolsets
            .entry(toolset)
            .or_insert_with(Vec::new)
            .push(name.clone());
        self.tools.insert(name.clone(), tool);
        debug!("Registered tool: {}", name);
    }

    /// Unregister a tool by name
    pub fn unregister(&mut self, name: &str) -> Option<Tool> {
        if let Some(tool) = self.tools.remove(name) {
            // Remove from toolset
            if let Some(tools) = self.toolsets.get_mut(&tool.toolset) {
                tools.retain(|n| n != name);
            }
            Some(tool)
        } else {
            None
        }
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// Dispatch a tool call - execute the handler with given arguments
    pub async fn dispatch(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| AgentError::ToolNotFound(name.to_string()))?;

        // Check if toolset is disabled
        if self.disabled_toolsets.contains(&tool.toolset) {
            return Err(AgentError::Tool(format!(
                "Toolset '{}' is disabled",
                tool.toolset
            )));
        }

        debug!("Dispatching tool: {}", name);
        let result = (tool.handler)(arguments).await?;
        Ok(result)
    }

    /// Get all enabled tool definitions (for sending to the model)
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .filter(|tool| !self.disabled_toolsets.contains(&tool.toolset))
            .map(|tool| tool.definition.clone())
            .collect()
    }

    /// Get all tool names
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Check if a tool exists
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get the number of registered tools
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Enable a toolset
    pub fn enable_toolset(&mut self, name: &str) {
        self.disabled_toolsets.retain(|n| n != name);
    }

    /// Disable a toolset
    pub fn disable_toolset(&mut self, name: &str) {
        if !self.disabled_toolsets.contains(&name.to_string()) {
            self.disabled_toolsets.push(name.to_string());
        }
    }

    /// Get all toolset names
    pub fn toolset_names(&self) -> Vec<String> {
        self.toolsets.keys().cloned().collect()
    }

    /// Get tools in a specific toolset
    pub fn toolset_tools(&self, toolset: &str) -> Vec<&Tool> {
        self.toolsets
            .get(toolset)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.tools.get(name))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Builder for creating tools with a fluent API
pub struct ToolBuilder {
    name: String,
    description: String,
    parameters: serde_json::Value,
    handler: Option<ToolHandler>,
    toolset: String,
    dangerous: bool,
}

impl ToolBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
            }),
            handler: None,
            toolset: "default".to_string(),
            dangerous: false,
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn parameters(mut self, params: serde_json::Value) -> Self {
        self.parameters = params;
        self
    }

    pub fn handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value>> + Send + 'static,
    {
        self.handler = Some(Arc::new(move |args| Box::pin(handler(args))));
        self
    }

    pub fn toolset(mut self, toolset: impl Into<String>) -> Self {
        self.toolset = toolset.into();
        self
    }

    pub fn dangerous(mut self, dangerous: bool) -> Self {
        self.dangerous = dangerous;
        self
    }

    pub fn build(self) -> Result<Tool> {
        let handler = self
            .handler
            .ok_or_else(|| AgentError::config("Tool handler is required"))?;

        Ok(Tool {
            definition: ToolDefinition {
                name: self.name,
                description: self.description,
                parameters: self.parameters,
            },
            handler,
            toolset: self.toolset,
            dangerous: self.dangerous,
        })
    }
}

/// Convenience function to create a tool builder
pub fn tool(name: impl Into<String>) -> ToolBuilder {
    ToolBuilder::new(name)
}
