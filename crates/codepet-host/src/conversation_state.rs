//! Host facade over Provider business read-state storage, routed to each plugin database.
use crate::{HostError, HostResult};
use codepet_gateway_sdk as gateway;
use codepet_provider_sdk as provider;
use codepet_provider_data::conversation_state::SharedConversationStateStore;
use std::path::{Path, PathBuf};

pub(crate) struct ConversationStateStore {
    shared: std::sync::Arc<SharedConversationStateStore>,
    stores: std::sync::Mutex<std::collections::BTreeMap<String, std::sync::Arc<SharedConversationStateStore>>>,
    path: Option<PathBuf>,
}

impl ConversationStateStore {
    pub(crate) fn memory() -> Self {
        Self {
            shared: std::sync::Arc::new(SharedConversationStateStore::memory()),
            stores: Default::default(),
            path: None,
        }
    }
    pub(crate) fn open(path: impl AsRef<Path>) -> HostResult<Self> {
        let path = if path.as_ref().is_absolute() {
            path.as_ref().to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let path=path.with_extension("sqlite");
        let shared = SharedConversationStateStore::open(&path).map_err(HostError::from)?;
        Ok(Self {
            shared: std::sync::Arc::new(shared),
            stores: Default::default(),
            path: Some(path),
        })
    }
    pub(crate) fn configure(&self, paths: Vec<(String,PathBuf)>) -> HostResult<()> {
        let mut stores=self.stores.lock().map_err(|_|HostError::new("conversation_state_unavailable","Store map poisoned"))?;
        for (id,path) in paths {
            if let std::collections::btree_map::Entry::Vacant(entry)=stores.entry(id) {
                entry.insert(std::sync::Arc::new(SharedConversationStateStore::open(path).map_err(HostError::from)?));
            }
        }
        Ok(())
    }
    fn store(&self, id: &str) -> HostResult<std::sync::Arc<SharedConversationStateStore>> {
        Ok(self.stores.lock().map_err(|_|HostError::new("conversation_state_unavailable","Store map poisoned"))?.get(id).cloned().unwrap_or_else(||self.shared.clone()))
    }
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub(crate) fn ensure_client(&self, scope: &str) -> HostResult<()> {
        for store in self.stores.lock().map_err(|_|HostError::new("conversation_state_unavailable","Store map poisoned"))?.values() { store.ensure_client(scope).map_err(HostError::from)?; }
        self.shared.ensure_client(scope).map_err(HostError::from)
    }
    pub(crate) fn decorate(
        &self,
        scope: &str,
        conversation: &mut gateway::Conversation,
    ) -> HostResult<()> {
        self.store(&conversation.resource.provider_id)?
            .decorate(scope, conversation)
            .map_err(HostError::from)
    }
    pub(crate) fn mark_read(
        &self,
        scope: &str,
        conversation: &gateway::RoutedResourceId,
        observed: &str,
    ) -> HostResult<gateway::ConversationReadState> {
        self.store(&conversation.provider_id)?
            .mark_read(scope, conversation, observed)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_summary(&self, conversation: &gateway::Conversation) -> HostResult<bool> {
        self.store(&conversation.resource.provider_id)?
            .observe_summary_changed(conversation)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_and_decorate_summaries(
        &self,
        scope: &str,
        conversations: &mut [gateway::Conversation],
    ) -> HostResult<bool> {
        let mut changed=false;
        for conversation in conversations { changed |= self.store(&conversation.resource.provider_id)?.observe_and_decorate_summaries(scope,std::slice::from_mut(conversation)).map_err(HostError::from)?; }
        Ok(changed)
    }
    pub(crate) fn observe_page_summaries(&self, scope: &str, conversations: &mut [gateway::Conversation]) -> HostResult<bool> {
        let mut changed = false;
        for conversation in conversations {
            changed |= self.store(&conversation.resource.provider_id)?.observe_page_summaries(scope, std::slice::from_mut(conversation)).map_err(HostError::from)?;
        }
        Ok(changed)
    }
    pub(crate) fn observe_detail(
        &self,
        conversation: &gateway::RoutedResourceId,
        items: &[gateway::ConversationItem],
    ) -> HostResult<Option<u64>> {
        self.store(&conversation.provider_id)?
            .observe_detail(conversation, items)
            .map_err(HostError::from)
    }
    pub(crate) fn observe_provider_event(
        &self,
        event: &provider::ProtocolEvent,
    ) -> HostResult<Option<(gateway::RoutedResourceId, u64)>> {
        let id=match event {
            provider::ProtocolEvent::EventConversationUpserted{params,..}=>Some(&params.conversation.resource.provider_id),
            provider::ProtocolEvent::EventConversationItemUpserted{params,..}=>params.conversation.as_ref().or_else(|| params.item.as_ref().map(codepet_provider_data::conversation_state::item_conversation)).map(|conversation| &conversation.provider_id),
            provider::ProtocolEvent::EventTurnUpserted{params,..}=>Some(&params.turn.conversation.provider_id),
            provider::ProtocolEvent::EventApprovalRequested{params,..}=>Some(&params.approval.conversation.provider_id),
            provider::ProtocolEvent::EventApprovalResolved{params,..}=>Some(&params.approval.conversation.provider_id),
            _=>None,
        };
        let store=match id{Some(id)=>self.store(id)?,None=>self.shared.clone()};
        store.observe_provider_event(event).map_err(HostError::from)
    }
}
