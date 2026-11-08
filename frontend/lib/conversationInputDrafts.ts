import type { InputQueueEnqueueRequest, TurnSelection } from '../../sdk/typescript/codepet-gateway-sdk/src/generated';
export interface ConversationDraft { text: string; selection: TurnSelection }
// Drafts last for this window session; only the Host stores the durable queue.
export const conversationDrafts = new Map<string, ConversationDraft>();
export const conversationDraftBackups = new Map<string, ConversationDraft[]>();
export const pendingQueueSubmissions = new Map<string, InputQueueEnqueueRequest>();
