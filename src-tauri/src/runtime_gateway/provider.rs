use super::generated::{
    ApprovalResolveRequest, ApprovalResolveResponse, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ProtocolFuture, Provider,
    TurnInterruptRequest, TurnInterruptResponse, TurnSendRequest, TurnSendResponse,
};

pub type ProviderFuture<'a, T> = ProtocolFuture<'a, T>;

pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> Provider;

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProviderFuture<'a, ConversationListResponse>;

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProviderFuture<'a, ConversationGetResponse>;

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProviderFuture<'a, ConversationCreateResponse>;

    fn turn_send<'a>(
        &'a self,
        request: TurnSendRequest,
    ) -> ProviderFuture<'a, TurnSendResponse>;

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProviderFuture<'a, TurnInterruptResponse>;

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProviderFuture<'a, ApprovalResolveResponse>;
}
