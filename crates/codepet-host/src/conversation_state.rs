//! Compatibility facade over the single Provider SDK read-state authority.
use crate::{HostError, HostResult};
use codepet_gateway_sdk as gateway;
use codepet_provider_sdk::{self as provider, conversation_state::SharedConversationStateStore};
use std::path::{Path, PathBuf};

pub(crate) struct ConversationStateStore {
    shared: SharedConversationStateStore,
    path: Option<PathBuf>,
}

impl ConversationStateStore {
    pub(crate) fn memory() -> Self {
        Self {
            shared: SharedConversationStateStore::memory(),
            path: None,
        }
    }
    pub(crate) fn open(path: impl AsRef<Path>) -> HostResult<Self> {
        let path = if path.as_ref().is_absolute() {
            path.as_ref().to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let shared = SharedConversationStateStore::open(&path).map_err(HostError::from)?;
        Ok(Self {
            shared,
            path: Some(path),
        })
    }
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub(crate) fn ensure_client(&self, scope: &str) -> HostResult<()> {
        self.shared.ensure_client(scope).map_err(HostError::from)
    }
    pub(crate) fn decorate(
        &self,
        scope: &str,
        conversation: &mut gateway::Conversation,
    ) -> HostResult<()> {
        self.shared
            .decorate(scope, conversation)
            .map_err(HostError::from)
    }
    pub(crate) fn mark_read(
        &self,
        scope: &str,
        conversation: &gateway::RoutedResourceId,
        observed: &str,
    ) -> HostResult<gateway::ConversationReadState> {
        self.shared
            .mark_read(scope, conversation, observed)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_summary(&self, conversation: &gateway::Conversation) -> HostResult<bool> {
        self.shared
            .observe_summary_changed(conversation)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_and_decorate_summaries(
        &self,
        scope: &str,
        conversations: &mut [gateway::Conversation],
    ) -> HostResult<bool> {
        self.shared
            .observe_and_decorate_summaries(scope, conversations)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_detail(
        &self,
        conversation: &gateway::RoutedResourceId,
        items: &[gateway::ConversationItem],
    ) -> HostResult<Option<u64>> {
        self.shared
            .observe_detail(conversation, items)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_provider_event(
        &self,
        event: &provider::ProtocolEvent,
    ) -> HostResult<Option<(gateway::RoutedResourceId, u64)>> {
        self.shared
            .observe_provider_event(event)
            .map_err(HostError::from)
    }
}
