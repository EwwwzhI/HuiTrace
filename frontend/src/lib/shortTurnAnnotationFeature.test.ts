import { describe, expect, it } from 'vitest';
import { isShortTurnAnnotationEnabled } from './shortTurnAnnotationFeature';

describe('short-turn annotation feature gate', () => {
  it('is visible in development', () => {
    expect(isShortTurnAnnotationEnabled('development', undefined)).toBe(true);
  });

  it('is visible with the explicit evaluation flag', () => {
    expect(isShortTurnAnnotationEnabled('production', 'true')).toBe(true);
  });

  it('is hidden in production by default', () => {
    expect(isShortTurnAnnotationEnabled('production', undefined)).toBe(false);
    expect(isShortTurnAnnotationEnabled('production', 'false')).toBe(false);
  });
});
