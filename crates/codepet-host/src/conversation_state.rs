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
    pub(crate) fn recent_turn_failures(&self, conversation: &gateway::RoutedResourceId) -> HostResult<Vec<gateway::TurnTask>> {
        self.store(&conversation.provider_id)?.recent_turn_failures(conversation).map_err(HostError::from)
    }

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
        self.observe_batches(scope, conversations, false)
    }
    pub(crate) fn observe_page_summaries(&self, scope: &str, conversations: &mut [gateway::Conversation]) -> HostResult<bool> {
        self.observe_batches(scope, conversations, true)
    }
    fn observe_batches(&self, scope: &str, conversations: &mut [gateway::Conversation], page: bool) -> HostResult<bool> {
        let observe = |store: &SharedConversationStateStore, rows: &mut [gateway::Conversation]| {
            if page { store.observe_page_summaries(scope, rows) }
            else { store.observe_and_decorate_summaries(scope, rows) }.map_err(HostError::from)
        };
        let Some(first) = conversations.first() else { return Ok(false); };
        if conversations.iter().all(|row| row.resource.provider_id == first.resource.provider_id) {
            return observe(self.store(&first.resource.provider_id)?.as_ref(), conversations);
        }
        // Mixed routes retain caller order while each Provider database gets
        // one transaction. Most list responses take the allocation-free path above.
        let mut batches = std::collections::BTreeMap::<String, Vec<(usize, gateway::Conversation)>>::new();
        for (index, row) in conversations.iter().enumerate() {
            batches.entry(row.resource.provider_id.clone()).or_default().push((index, row.clone()));
        }
        let mut changed = false;
        for (provider, batch) in batches {
            let (indices, mut rows): (Vec<_>, Vec<_>) = batch.into_iter().unzip();
            changed |= observe(self.store(&provider)?.as_ref(), &mut rows)?;
            for (index, row) in indices.into_iter().zip(rows) { conversations[index] = row; }
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
            provider::ProtocolEvent::EventConversationItemUpserted{params,..}=>params.conversation.as_ref().or_else(|| params.item.as_ref().and_then(codepet_provider_data::conversation_state::item_conversation)).map(|conversation| &conversation.provider_id),
            provider::ProtocolEvent::EventTurnUpserted{params,..}=>Some(&params.turn.conversation.provider_id),
            provider::ProtocolEvent::EventApprovalRequested{params,..}=>Some(&params.approval.conversation.provider_id),
            provider::ProtocolEvent::EventApprovalResolved{params,..}=>Some(&params.approval.conversation.provider_id),
            _=>None,
        };
        let store=match id{Some(id)=>self.store(id)?,None=>self.shared.clone()};
        store.observe_provider_event(event).map_err(HostError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(provider: &str, id: &str, preview: &str) -> gateway::Conversation {
        serde_json::from_value(serde_json::json!({
            "resource":{"providerId":provider,"nativeResourceId":id},
            "title":id,"preview":preview,"status":"idle","updatedAt":1,
        })).unwrap()
    }

    #[test]
    fn batches_preserve_provider_routing_order_and_reader_isolation() {
        let store = ConversationStateStore::memory();
        for provider in ["p1", "p2"] {
            let shared = std::sync::Arc::new(SharedConversationStateStore::memory());
            shared.ensure_client("reader-a").unwrap();
            shared.ensure_client("reader-b").unwrap();
            store.stores.lock().unwrap().insert(provider.into(), shared);
        }
        let mut rows = vec![row("p1", "same-id", "before"), row("p2", "same-id", "before"), row("p1", "other-id", "before")];
        assert!(!store.observe_page_summaries("reader-a", &mut rows).unwrap(), "new baselines do not invalidate the page");
        assert!(rows.iter().all(|row| !row.read_state.as_ref().unwrap().unread));
        for row in &mut rows { row.preview = Some("after".into()); }
        assert!(store.observe_and_decorate_summaries("reader-a", &mut rows).unwrap());
        assert!(rows.iter().all(|row| row.read_state.as_ref().unwrap().unread));
        assert_eq!(rows.iter().map(|row| row.resource.provider_id.as_str()).collect::<Vec<_>>(), vec!["p1", "p2", "p1"]);
        let version = rows[0].read_state.as_ref().unwrap().activity_version.clone();
        store.mark_read("reader-a", &rows[0].resource, &version).unwrap();
        assert!(!store.observe_page_summaries("reader-a", &mut rows).unwrap());
        assert!(!rows[0].read_state.as_ref().unwrap().unread);
        assert!(rows[1].read_state.as_ref().unwrap().unread, "same native ID in another Provider is independent");
        store.observe_page_summaries("reader-b", &mut rows).unwrap();
        assert!(rows[0].read_state.as_ref().unwrap().unread, "another reader remains unread");
        // The common single-Provider path and mixed batches use the same state.
        assert!(!store.observe_and_decorate_summaries("reader-b", &mut rows[..1]).unwrap());
        assert!(!store.observe_page_summaries("reader-a", &mut []).unwrap());
    }
}
