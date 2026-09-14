export function durationBucket(duration: number) {
  if (duration >= 100 && duration <= 299) return '100–299 ms';
  if (duration >= 300 && duration <= 499) return '300–499 ms';
  if (duration >= 500 && duration <= 799) return '500–799 ms';
  if (duration >= 800 && duration <= 1200) return '800–1200 ms';
  return 'non-short control';
}

export function shouldCreateRegion(syncing: boolean, eventIds: readonly string[], regionId: string) {
  return !syncing && !eventIds.includes(regionId);
}

export function saveLabel(editRevision: number, savedRevision: number, savingRevision: number | null, failed: boolean) {
  if (failed) return 'Save failed';
  if (savingRevision !== null) return `Saving r${savingRevision}…`;
  return editRevision === savedRevision ? 'Saved locally' : 'Unsaved';
}

type WindowBounds = { window_id: string; source_start_ms: number; source_end_ms: number };
type EventBounds = { start_ms: number; end_ms: number; annotation_status: string };

export function pendingInWindow(events: readonly EventBounds[], window: WindowBounds, review: boolean) {
  return events.filter(event => event.start_ms < window.source_end_ms && event.end_ms > window.source_start_ms
    && (event.annotation_status === 'pending' || (review && event.annotation_status === 'review_pending'))).length;
}

export function reviewCompletion(windows: readonly WindowBounds[], statuses: Record<string, string>) {
  const ids = [...new Set(windows.map(window => window.window_id))];
  const completed = ids.filter(id => statuses[id] === 'reviewed_second_pass').length;
  return { completed, total: ids.length, complete: ids.length > 0 && completed === ids.length };
}
