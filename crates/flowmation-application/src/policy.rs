use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use flowmation_domain::tool::{ToolEffect, ToolPermissionMode};
use serde_json::{Map, Value};

use crate::tool::ExecutionMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationDecision {
    Allow,
    Deny,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub arguments: Map<String, Value>,
    pub effect: ToolEffect,
    pub permission_mode: ToolPermissionMode,
    pub execution_mode: ExecutionMode,
}

#[async_trait]
pub trait PermissionBroker: Debug + Send + Sync {
    async fn request(&self, request: PermissionRequest) -> AuthorizationDecision;
}

#[async_trait]
pub trait AuthorizationPolicy: Debug + Send + Sync {
    async fn authorize(&self, request: PermissionRequest) -> AuthorizationDecision;
}

#[derive(Debug)]
pub struct StandardAuthorizationPolicy {
    broker: Arc<dyn PermissionBroker>,
}

impl StandardAuthorizationPolicy {
    #[must_use]
    pub fn new(broker: Arc<dyn PermissionBroker>) -> Self {
        Self { broker }
    }
}

#[async_trait]
impl AuthorizationPolicy for StandardAuthorizationPolicy {
    async fn authorize(&self, request: PermissionRequest) -> AuthorizationDecision {
        if request.execution_mode == ExecutionMode::Scheduled {
            return if request.effect == ToolEffect::Read
                && request.permission_mode == ToolPermissionMode::Effect
            {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Deny
            };
        }
        if request.effect == ToolEffect::Read
            || request.permission_mode == ToolPermissionMode::SelfManaged
        {
            return AuthorizationDecision::Allow;
        }
        self.broker.request(request).await
    }
}

#[derive(Debug)]
pub struct FixedPermissionBroker {
    decision: AuthorizationDecision,
}

impl FixedPermissionBroker {
    #[must_use]
    pub fn new(decision: AuthorizationDecision) -> Self {
        Self { decision }
    }
}

#[async_trait]
impl PermissionBroker for FixedPermissionBroker {
    async fn request(&self, _request: PermissionRequest) -> AuthorizationDecision {
        self.decision
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::Map;

    use super::{
        AuthorizationDecision, AuthorizationPolicy, FixedPermissionBroker, PermissionRequest,
        StandardAuthorizationPolicy,
    };
    use crate::tool::ExecutionMode;
    use flowmation_domain::tool::{ToolEffect, ToolPermissionMode};

    fn request(
        effect: ToolEffect,
        permission_mode: ToolPermissionMode,
        execution_mode: ExecutionMode,
    ) -> PermissionRequest {
        PermissionRequest {
            tool_name: "test".to_owned(),
            arguments: Map::new(),
            effect,
            permission_mode,
            execution_mode,
        }
    }

    #[tokio::test]
    async fn automatically_allows_reads_and_denies_scheduled_effects() {
        let policy = StandardAuthorizationPolicy::new(Arc::new(FixedPermissionBroker::new(
            AuthorizationDecision::Allow,
        )));
        assert_eq!(
            policy
                .authorize(request(
                    ToolEffect::Read,
                    ToolPermissionMode::Effect,
                    ExecutionMode::Scheduled
                ))
                .await,
            AuthorizationDecision::Allow
        );
        assert_eq!(
            policy
                .authorize(request(
                    ToolEffect::Write,
                    ToolPermissionMode::Effect,
                    ExecutionMode::Scheduled
                ))
                .await,
            AuthorizationDecision::Deny
        );
        assert_eq!(
            policy
                .authorize(request(
                    ToolEffect::External,
                    ToolPermissionMode::SelfManaged,
                    ExecutionMode::Scheduled
                ))
                .await,
            AuthorizationDecision::Deny
        );
    }

    #[tokio::test]
    async fn self_managed_tools_are_allowed_outside_scheduled_runs() {
        let policy = StandardAuthorizationPolicy::new(Arc::new(FixedPermissionBroker::new(
            AuthorizationDecision::Deny,
        )));
        assert_eq!(
            policy
                .authorize(request(
                    ToolEffect::External,
                    ToolPermissionMode::SelfManaged,
                    ExecutionMode::Direct
                ))
                .await,
            AuthorizationDecision::Allow
        );
    }
}
