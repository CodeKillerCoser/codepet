import { expect, it } from 'vitest';
import { providerKindsFromFeatures } from './providerPresentation';
it('shows combined explicit functions and treats missing or future declarations conservatively', () => {
  expect(providerKindsFromFeatures(['agent', 'observation', 'runtimeInventory'])).toEqual(['agent', 'observation']);
  expect(providerKindsFromFeatures(['observation'])).toEqual(['observation']);
  for (const features of [undefined, null, [], ['future'], ['turn.send']]) expect(providerKindsFromFeatures(features)).toEqual(['extension']);
});
