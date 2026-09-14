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
