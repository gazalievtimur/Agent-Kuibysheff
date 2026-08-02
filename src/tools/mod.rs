pub mod error;
pub mod fs_home;
pub mod local_tools;
pub mod registry;

pub use error::{ExternalToolError, HomeFsError, LocalToolsError, PolicyError, ToolError};
pub use registry::{BuiltinHandlerId, BuiltinPrompt, ToolDescriptor, BUILTINS};

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tracing::warn;

use crate::access::EffectiveToolPolicy;
use crate::tool_api::ToolExecutor;

use self::fs_home::HomeFs;
use self::local_tools::LocalTools;
use self::registry::known_builtin_names;

pub struct CompositeToolExecutor {
    home: HomeFs,
    local_tools: LocalTools,
    external: Arc<dyn ToolExecutor>,
}

impl CompositeToolExecutor {
    #[must_use]
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
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        match registry::handler_for_server(server) {
            Some(BuiltinHandlerId::Home) => self
                .home
                .call(tool, arguments)
                .await
                .map_err(ToolError::from),
            Some(BuiltinHandlerId::LocalTools) => self
                .local_tools
                .call(tool, arguments)
                .await
                .map_err(ToolError::from),
            None => self.external.call_tool(server, tool, arguments).await,
        }
    }

    fn available_tools(&self) -> Vec<String> {
        let mut tools = self.external.available_tools();
        tools.extend(known_builtin_names().map(str::to_string));
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
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        let qualified = format!("{server}.{tool}");
        if !self.policy.allows_server_tool(server, tool) {
            warn!(
                capability = %qualified,
                decision = "deny",
                "tool call denied by access policy"
            );
            return Err(ToolError::Policy(PolicyError::ToolDenied {
                tool: qualified,
            }));
        }
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
    use crate::access::{known_builtins, EffectiveToolPolicy, QualifiedTool, ResolvedAccessPolicy};
    use std::collections::BTreeSet;

    #[test]
    fn registry_names_match_access_known_and_composite_availability() {
        let registry: Vec<_> = known_builtin_names().collect();
        let access_known: Vec<_> = known_builtins().collect();
        assert_eq!(registry, access_known);

        // Policy-known set used at compile time is exactly the registry.
        let access = ResolvedAccessPolicy::legacy();
        let skills: BTreeSet<_> = registry
            .iter()
            .filter_map(|name| QualifiedTool::parse(name).ok())
            .collect();
        let policy = EffectiveToolPolicy::compile(&access, &skills, []);
        let advertised: BTreeSet<_> = policy.advertised().into_iter().collect();
        let legacy_expected: BTreeSet<_> = registry::legacy_builtin_names()
            .map(str::to_string)
            .collect();
        assert_eq!(advertised, legacy_expected);
    }

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
        ) -> Result<Value, ToolError> {
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
        let skills = BTreeSet::from([
            QualifiedTool::parse("home.read").unwrap(),
            QualifiedTool::parse("docs.search").unwrap(),
        ]);
        let mcp = BTreeSet::from([
            QualifiedTool::parse("docs.search").unwrap(),
            QualifiedTool::parse("docs.secret").unwrap(),
        ]);
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
        assert!(matches!(
            denied,
            ToolError::Policy(PolicyError::ToolDenied { .. })
        ));

        let mcp_denied = executor
            .call_tool("docs", "secret", serde_json::json!({}))
            .await
            .expect_err("mcp denied when not in skills");
        assert!(matches!(
            mcp_denied,
            ToolError::Policy(PolicyError::ToolDenied { .. })
        ));

        assert_eq!(
            inner.called.lock().expect("lock").as_slice(),
            ["home.read".to_string()]
        );
    }
}
