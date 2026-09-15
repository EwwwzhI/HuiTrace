import { describe, expect, it } from 'vitest';
import { canShowReconstructionEvidence } from './utteranceAnnotationVisibility';

describe('utterance annotation evidence visibility', () => {
  it('never exposes system evidence in Blind', () => {
    expect(canShowReconstructionEvidence('blind', true)).toBe(false);
  });

  it('unlocks system evidence only in Review after Blind completion', () => {
    expect(canShowReconstructionEvidence('review', false)).toBe(false);
    expect(canShowReconstructionEvidence('review', true)).toBe(true);
  });
});
