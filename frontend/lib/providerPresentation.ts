export type ProviderKind = 'agent' | 'observation';
export const providerKindLabel = (kind: ProviderKind) => ({ agent: '智能体', observation: '通知助手' })[kind];
/** A plugin may declare both supported functions. Missing features create no invented category. */
export function providerKindsFromFeatures(features?: readonly string[] | null): ProviderKind[] {
  const kinds: ProviderKind[] = [];
  if (features?.includes('agent')) kinds.push('agent');
  if (features?.includes('observation')) kinds.push('observation');
  return kinds;
}
