export type ProviderKind = 'agent' | 'observation' | 'extension';
export const providerKindLabel = (kind: ProviderKind) => ({ agent: '智能体', observation: '通知观测', extension: '扩展服务' })[kind];
/** Presentation uses explicit plugin features, independently of instance methods. */
export function providerKindsFromFeatures(features?: readonly string[] | null): ProviderKind[] {
  const kinds: ProviderKind[] = [];
  if (features?.includes('agent')) kinds.push('agent');
  if (features?.includes('observation')) kinds.push('observation');
  return kinds.length ? kinds : ['extension'];
}
