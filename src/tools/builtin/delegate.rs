//! delegate_task — port of hermes' tools/delegate_tool.py
//!
//! Spawns one or more sub-agents in isolated contexts. The actual agent
//! execution is provided by the `SubAgentRunner` installed on the context
//! (implemented by `Agent`), keeping tools/ free of an agent dependency.

use crate::tools::{tool, ToolRegistry};
use serde_json::json;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(delegate_task_tool());
}

fn delegate_task_tool() -> crate::tools::Tool {
    tool("delegate_task")
        .description(
            "Spawn one or more subagents in isolated contexts. Each subagent gets a fresh \
             conversation with no knowledge of your current chat — give it a self-contained \
             goal and all context it needs. Use for parallelizable research or independent \
             subtasks. Results come back as one combined report.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string", "description": "What the subagent should accomplish. Be specific and self-contained."},
                "context": {"type": "string", "description": "Background information the subagent needs: file paths, error messages, project structure, constraints."},
                "tasks": {
                    "type": "array",
                    "description": "Multiple tasks to run as parallel subagents (alternative to single goal).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "goal": {"type": "string", "description": "Task goal"},
                            "context": {"type": "string", "description": "Task-specific context"}
                        },
                        "required": ["goal"]
                    }
                }
            },
            "required": []
        }))
        .handler(|args, ctx| async move {
            let Some(runner) = ctx.subagent_runner() else {
                return Ok(json!({
                    "success": false,
                    "error": "Delegation is not available in this run (no sub-agent runner wired)."
                }));
            };

            let max_children = ctx.config.delegation.max_concurrent_children.max(1);

            // Collect (goal, context) pairs.
            let mut tasks: Vec<(String, String)> = Vec::new();
            if let Some(list) = args.get("tasks").and_then(|v| v.as_array()) {
                for item in list {
                    let goal = item.get("goal").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if goal.is_empty() {
                        continue;
                    }
                    let context = item.get("context").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    tasks.push((goal, context));
                }
            }
            if tasks.is_empty() {
                let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if goal.is_empty() {
                    return Ok(json!({"success": false, "error": "delegate_task: provide 'goal' or 'tasks'"}));
                }
                let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("").to_string();
                tasks.push((goal, context));
            }
            if tasks.len() > max_children {
                return Ok(json!({
                    "success": false,
                    "error": format!(
                        "delegation.max_concurrent_children is {} but {} tasks were requested. Reduce the task count.",
                        max_children, tasks.len()
                    )
                }));
            }

            // Run sub-agents (bounded concurrency = max_children, which is the
            // validated cap, so a simple join_all is fine).
            let mut handles = Vec::new();
            for (goal, context) in tasks.clone() {
                let runner = runner.clone();
                handles.push(tokio::spawn(async move {
                    runner.run_subagent(&goal, &context).await
                }));
            }

            let mut results = Vec::new();
            let mut failed = 0usize;
            for (idx, handle) in handles.into_iter().enumerate() {
                let (goal, _) = &tasks[idx];
                match handle.await {
                    Ok(Ok(answer)) => results.push(json!({
                        "task": goal,
                        "status": "completed",
                        "result": answer,
                    })),
                    Ok(Err(e)) => {
                        failed += 1;
                        results.push(json!({"task": goal, "status": "error", "error": e.to_string()}));
                    }
                    Err(e) => {
                        failed += 1;
                        results.push(json!({"task": goal, "status": "error", "error": format!("join: {}", e)}));
                    }
                }
            }

            Ok(json!({
                "success": failed == 0,
                "subagents": results.len(),
                "failed": failed,
                "results": results,
            }))
        })
        .toolset("delegation")
        .emoji("🤝")
        .build()
        .expect("delegate_task builds")
}
