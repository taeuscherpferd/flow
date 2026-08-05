use async_trait::async_trait;
use flowmation_application::{AuthorizationDecision, PermissionBroker, PermissionRequest};
use serde_json::Value;
use tokio::sync::Mutex;

#[async_trait]
pub trait PermissionPrompt: Send + Sync {
    async fn confirm(&self, prompt: &str, details: &str) -> Result<Option<String>, String>;
}

#[derive(Debug)]
pub struct SerializedPermissionBroker<P> {
    prompt: P,
    queue: Mutex<()>,
}

impl<P> SerializedPermissionBroker<P> {
    #[must_use]
    pub const fn new(prompt: P) -> Self {
        Self {
            prompt,
            queue: Mutex::const_new(()),
        }
    }
}

#[async_trait]
impl<P> PermissionBroker for SerializedPermissionBroker<P>
where
    P: PermissionPrompt + std::fmt::Debug,
{
    async fn request(&self, request: PermissionRequest) -> AuthorizationDecision {
        let _turn = self.queue.lock().await;
        let details = format!(
            "{} requests {:?} permission:\n{}",
            request.tool_name,
            request.effect,
            Value::Object(request.arguments)
        );
        let answer = self
            .prompt
            .confirm(&format!("Allow {}?", request.tool_name), &details)
            .await;
        if matches!(
            answer,
            Ok(Some(answer)) if matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "y" | "yes"
            )
        ) {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Duration;

    use flowmation_application::{AuthorizationDecision, PermissionBroker, PermissionRequest};
    use flowmation_domain::tool::{ToolEffect, ToolPermissionMode};
    use serde_json::Map;
    use tokio::sync::{Mutex, mpsc, oneshot};

    use super::{PermissionPrompt, SerializedPermissionBroker};

    struct PendingConfirmation {
        prompt: String,
        response: oneshot::Sender<Option<String>>,
    }

    #[derive(Debug)]
    struct ControlledPrompt {
        pending: mpsc::UnboundedSender<PendingConfirmation>,
        responses: Mutex<VecDeque<oneshot::Receiver<Option<String>>>>,
    }

    #[async_trait::async_trait]
    impl PermissionPrompt for ControlledPrompt {
        async fn confirm(&self, prompt: &str, _details: &str) -> Result<Option<String>, String> {
            let (sender, receiver) = oneshot::channel();
            self.pending
                .send(PendingConfirmation {
                    prompt: prompt.to_owned(),
                    response: sender,
                })
                .map_err(|error| error.to_string())?;
            self.responses.lock().await.push_back(receiver);
            let receiver = self
                .responses
                .lock()
                .await
                .pop_front()
                .ok_or_else(|| "confirmation response queue was empty".to_owned())?;
            receiver.await.map_err(|error| error.to_string())
        }
    }

    fn request(name: &str) -> PermissionRequest {
        PermissionRequest {
            tool_name: name.to_owned(),
            arguments: Map::new(),
            effect: ToolEffect::External,
            permission_mode: ToolPermissionMode::Effect,
            execution_mode: flowmation_application::ExecutionMode::Direct,
        }
    }

    #[tokio::test]
    async fn serializes_concurrent_permission_confirmations()
    -> Result<(), Box<dyn std::error::Error>> {
        let (pending_sender, mut pending_receiver) = mpsc::unbounded_channel();
        let broker = Arc::new(SerializedPermissionBroker::new(ControlledPrompt {
            pending: pending_sender,
            responses: Mutex::new(VecDeque::new()),
        }));
        let first_broker = Arc::clone(&broker);
        let first = tokio::spawn(async move { first_broker.request(request("first")).await });
        let second_broker = Arc::clone(&broker);
        let second = tokio::spawn(async move { second_broker.request(request("second")).await });

        let first_pending = pending_receiver
            .recv()
            .await
            .ok_or("first prompt did not start")?;
        assert_eq!(first_pending.prompt, "Allow first?");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), pending_receiver.recv())
                .await
                .is_err()
        );
        first_pending
            .response
            .send(Some("yes".to_owned()))
            .map_err(|_| "first prompt receiver closed")?;
        assert_eq!(first.await?, AuthorizationDecision::Allow);

        let second_pending = pending_receiver
            .recv()
            .await
            .ok_or("second prompt did not start")?;
        assert_eq!(second_pending.prompt, "Allow second?");
        second_pending
            .response
            .send(Some("no".to_owned()))
            .map_err(|_| "second prompt receiver closed")?;
        assert_eq!(second.await?, AuthorizationDecision::Deny);
        Ok(())
    }
}
