import { describe, expect, it } from 'vitest';
import { durationBucket, saveLabel, shouldCreateRegion } from './annotationIntegrity';

describe('annotation workspace integrity helpers', () => {
  it.each([[50,'non-short control'],[99,'non-short control'],[100,'100–299 ms'],[299,'100–299 ms'],[300,'300–499 ms'],[499,'300–499 ms'],[500,'500–799 ms'],[799,'500–799 ms'],[800,'800–1200 ms'],[1200,'800–1200 ms'],[1201,'non-short control']] as const)('classifies %ims', (duration, bucket) => expect(durationBucket(duration)).toBe(bucket));
  it('suppresses programmatic and existing WaveSurfer regions', () => {
    expect(shouldCreateRegion(true, [], 'temporary')).toBe(false);
    expect(shouldCreateRegion(false, ['meeting-event-0001'], 'meeting-event-0001')).toBe(false);
    expect(shouldCreateRegion(false, ['meeting-event-0001'], 'temporary')).toBe(true);
  });
  it('does not report Saved while a newer edit is outstanding', () => {
    expect(saveLabel(2, 1, null, false)).toBe('Unsaved');
    expect(saveLabel(2, 1, 1, false)).toBe('Saving r1…');
    expect(saveLabel(2, 2, null, false)).toBe('Saved locally');
  });
});
