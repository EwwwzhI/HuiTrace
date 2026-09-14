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


it('blocks pending annotations in every overlapping window, excluding boundary contact', async () => {
  const { pendingInWindow, reviewCompletion } = await import('./annotationIntegrity');
  const events = [{ start_ms: 4500, end_ms: 4800, annotation_status: 'pending' }];
  const a = { window_id: 'a', source_start_ms: 0, source_end_ms: 5000 };
  const b = { window_id: 'b', source_start_ms: 4000, source_end_ms: 9000 };
  expect(pendingInWindow(events, a, false)).toBe(1);
  expect(pendingInWindow(events, b, false)).toBe(1);
  expect(pendingInWindow(events, { ...b, source_start_ms: 4800 }, false)).toBe(0);
  events[0].annotation_status = 'blind_confirmed';
  expect(pendingInWindow(events, a, false)).toBe(0);
  expect(pendingInWindow(events, b, false)).toBe(0);
  events[0].annotation_status = 'review_pending';
  expect(pendingInWindow(events, a, true)).toBe(1);
  expect(reviewCompletion([a, b], { a: 'reviewed_second_pass' }).complete).toBe(false);
  expect(reviewCompletion([a, b], { a: 'reviewed_second_pass', b: 'reviewed_second_pass' }).complete).toBe(true);
  expect(reviewCompletion([], {}).complete).toBe(false);
});
