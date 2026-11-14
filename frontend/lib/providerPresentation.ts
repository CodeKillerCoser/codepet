export type ProviderKind = 'agent' | 'observation' | 'extension';
export const providerKindLabel = (kind: ProviderKind) => ({ agent: '智能体', observation: '通知观测', extension: '扩展服务' })[kind];
/** Classify declared capabilities, never the plugin name or runtime requirement. */
export function providerKindFromMethods(methods: readonly string[]): ProviderKind {
  if (methods.some(method => method.startsWith('conversation.') || method.startsWith('turn.'))) return 'agent';
  if (methods.some(method => method.startsWith('observation.') || method.startsWith('notification.'))) return 'observation';
  return 'extension';
}
