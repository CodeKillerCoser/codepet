import { expect, it } from 'vitest';
import { providerKindFromMethods } from './providerPresentation';
it('uses capabilities instead of runtime requirements to distinguish provider roles', () => {
  expect(providerKindFromMethods(['turn.send'])).toBe('agent');
  expect(providerKindFromMethods(['observation.subscribe'])).toBe('observation');
  expect(providerKindFromMethods(['notification.read'])).toBe('observation');
  expect(providerKindFromMethods([])).toBe('extension');
  expect(providerKindFromMethods(['codepet.usage.query'])).toBe('extension');
});
