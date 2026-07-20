pub mod fs_home;
pub mod local_tools;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tracing::warn;

use crate::access::EffectiveToolPolicy;
use crate::mcp::{Error, ToolExecutor};

use self::fs_home::HomeFs;
use self::local_tools::LocalTools;

pub struct CompositeToolExecutor {
    home: HomeFs,
    local_tools: LocalTools,
    external: Arc<dyn ToolExecutor>,
}

impl CompositeToolExecutor {
    pub fn new(home: HomeFs, local_tools: LocalTools, external: Arc<dyn ToolExecutor>) -> Self {
        Self {
            home,
            local_tools,
            external,
        }
    }
}

#[async_trait]
impl ToolExecutor for CompositeToolExecutor {
    async fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value, Error> {
        match server {
            "home" => self.home.call(tool, arguments).await,
            "local_tools" => self.local_tools.call(tool, arguments).await,
            _ => self.external.call_tool(server, tool, arguments).await,
        }
    }

    fn available_tools(&self) -> Vec<String> {
        let mut tools = self.external.available_tools();
        tools.extend(
            [
                "home.list",
                "home.read",
                "home.write",
                "home.run",
                "local_tools.search_docs",
                "local_tools.read_file",
            ]
            .into_iter()
            .map(str::to_string),
        );
        tools.sort();
        tools.dedup();
        tools
    }
}

/// Enforcing gate over any [`ToolExecutor`]: advertises and dispatches only the effective set.
pub struct PolicyToolExecutor {
    inner: Arc<dyn ToolExecutor>,
    policy: EffectiveToolPolicy,
}

impl PolicyToolExecutor {
    #[must_use]
    pub fn new(inner: Arc<dyn ToolExecutor>, policy: EffectiveToolPolicy) -> Self {
        Self { inner, policy }
    }

    #[must_use]
    pub fn policy(&self) -> &EffectiveToolPolicy {
        &self.policy
    }
}

#[async_trait]
impl ToolExecutor for PolicyToolExecutor {
    async fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value, Error> {
        if !self.policy.allows_server_tool(server, tool) {
            let qualified = format!("{server}.{tool}");
            warn!(
                capability = %qualified,
                decision = "deny",
                "tool call denied by access policy"
            );
            return Err(Error::PolicyDenied { tool: qualified });
        }
        let qualified = format!("{server}.{tool}");
        tracing::info!(
            capability = %qualified,
            decision = "allow",
            "tool call allowed by access policy"
        );
        self.inner.call_tool(server, tool, arguments).await
    }

    fn available_tools(&self) -> Vec<String> {
        self.policy.advertised()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{EffectiveToolPolicy, QualifiedTool, ResolvedAccessPolicy};
    use std::collections::BTreeSet;

    struct RecordingTools {
        called: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolExecutor for RecordingTools {
        async fn call_tool(
            &self,
            server: &str,
            tool: &str,
            _arguments: Value,
        ) -> Result<Value, Error> {
            self.called
                .lock()
                .expect("lock")
                .push(format!("{server}.{tool}"));
            Ok(serde_json::json!({"ok": true}))
        }

        fn available_tools(&self) -> Vec<String> {
            vec![
                "home.read".to_string(),
                "home.write".to_string(),
                "docs.search".to_string(),
            ]
        }
    }

    #[tokio::test]
    async fn policy_executor_hides_and_denies_disallowed_tools() {
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([QualifiedTool::parse("home.read").unwrap()]);
        let mcp = BTreeSet::from([QualifiedTool::parse("docs.search").unwrap()]);
        let policy = EffectiveToolPolicy::compile(&access, &skills, mcp);
        let inner = Arc::new(RecordingTools {
            called: std::sync::Mutex::new(Vec::new()),
        });
        let executor = PolicyToolExecutor::new(inner.clone(), policy);

        assert_eq!(
            executor.available_tools(),
            vec!["docs.search".to_string(), "home.read".to_string()]
        );

        executor
            .call_tool("home", "read", serde_json::json!({}))
            .await
            .expect("allowed");
        let denied = executor
            .call_tool("home", "write", serde_json::json!({}))
            .await
            .expect_err("denied");
        assert!(matches!(denied, Error::PolicyDenied { .. }));
        assert_eq!(
            inner.called.lock().expect("lock").as_slice(),
            ["home.read".to_string()]
        );
    }
}
