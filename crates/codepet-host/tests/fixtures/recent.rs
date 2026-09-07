use super::*;
use codepet_provider_sdk::{
    self as sdk, conversation_query::SnapshotPager,
    conversation_state::SharedConversationStateStore,
};

#[derive(Default)]
pub(super) struct RecentFixture {
    active: SnapshotPager<sdk::ConversationActiveEntry>,
    unread: SnapshotPager<sdk::ConversationUnreadEntry>,
    summaries: SnapshotPager<Conversation>,
}

pub(super) fn enabled() -> bool {
    std::env::var_os("CODEPET_FAKE_RECENT").is_some()
}

impl RecentFixture {
    pub(super) fn active(
        &self,
        request: sdk::ConversationActiveListRequest,
    ) -> Result<sdk::ConversationActiveListResponse, sdk::ProtocolError> {
        if std::env::var_os("CODEPET_FAKE_RECENT_ACTIVE_FAIL").is_some()
            || std::env::var_os("CODEPET_FAKE_RECENT_ACTIVE_FAIL_FILE")
                .is_some_and(|path| std::path::Path::new(&path).exists())
        {
            return Err(protocol_error(
                "conversation_query_incomplete",
                "injected active enumeration failure",
            ));
        }
        let binding = format!("active:{}", request.route.provider_instance_id);
        let page = if let Some(cursor) = request.cursor {
            self.active.page(&binding, &cursor, request.limit)?
        } else {
            let rows = (0..125)
                .map(|n| sdk::ConversationActiveEntry {
                    conversation: provider_resource(&request.route, &format!("active-{n:03}")),
                    status: ConversationStatus::Running,
                    activity_version: "native-active-1".into(),
                })
                .collect();
            self.active.start(binding, rows, request.limit)?
        };
        Ok(sdk::ConversationActiveListResponse {
            conversations: page.rows,
            revision: page.revision,
            page_info: PageInfo {
                next_cursor: page.next_cursor,
            },
        })
    }

    pub(super) fn unread(
        &self,
        request: sdk::ConversationUnreadListRequest,
    ) -> Result<sdk::ConversationUnreadListResponse, sdk::ProtocolError> {
        let binding = format!(
            "unread:{}:{}",
            request.route.provider_instance_id, request.reader_scope
        );
        let page = if let Some(cursor) = request.cursor {
            self.unread.page(&binding, &cursor, request.limit)?
        } else {
            let rows = SharedConversationStateStore::from_env()?
                .unread(&request.reader_scope, &request.route.provider_instance_id)?
                .into_iter()
                .map(|(conversation, read_state)| sdk::ConversationUnreadEntry {
                    conversation: provider_resource(
                        &request.route,
                        &conversation.native_resource_id,
                    ),
                    read_state,
                })
                .collect();
            self.unread.start(binding, rows, request.limit)?
        };
        Ok(sdk::ConversationUnreadListResponse {
            conversations: page.rows,
            revision: page.revision,
            page_info: PageInfo {
                next_cursor: page.next_cursor,
            },
        })
    }

    pub(super) fn summaries(
        &self,
        request: ConversationListRequest,
    ) -> Result<ConversationListResponse, sdk::ProtocolError> {
        let binding =
            serde_json::to_string(&(&request.route, &request.query, &request.reader_scope))
                .unwrap();
        let page = if let Some(cursor) = request.cursor {
            self.summaries.page(&binding, &cursor, request.limit)?
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let mut rows: Vec<_> = (0..125)
                .map(|n| {
                    let mut row = conversation(&request.route, &format!("active-{n:03}"));
                    row.status = ConversationStatus::Running;
                    row.updated_at = Some(1);
                    row
                })
                .collect();
            let mut old = conversation(&request.route, "unread-old");
            old.updated_at = Some(1);
            rows.push(old);
            rows.push(conversation(&request.route, "conversation-list"));
            let mut dated = conversation(&request.route, "recent");
            dated.updated_at = Some(now);
            rows.push(dated);
            rows.retain(|row| match request.query.as_ref().unwrap() {
                sdk::ConversationListQuery::ConversationUpdatedAfterQuery(query) => {
                    row.updated_at.is_some_and(|t| t >= query.updated_after)
                }
                sdk::ConversationListQuery::ConversationIdsQuery(query) => {
                    query.ids.contains(&row.resource.native_resource_id)
                }
            });
            rows.sort_by(|a, b| {
                b.updated_at.cmp(&a.updated_at).then_with(|| {
                    a.resource
                        .native_resource_id
                        .cmp(&b.resource.native_resource_id)
                })
            });
            if let Some(scope) = &request.reader_scope {
                SharedConversationStateStore::from_env()?.decorate_many(scope, &mut rows)?;
            }
            self.summaries.start(binding, rows, request.limit)?
        };
        Ok(ConversationListResponse {
            conversations: page.rows,
            page_info: PageInfo {
                next_cursor: page.next_cursor,
            },
        })
    }
}
