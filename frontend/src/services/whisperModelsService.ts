import { WhisperAPI, type ModelInfo } from '@/lib/whisper';

// Share only in-flight scans, including across settings remounts. Never cache
// completed results: downloaded/deleted/replaced files must be checked again.
let pending: Promise<ModelInfo[]> | undefined;

export function loadWhisperModels(): Promise<ModelInfo[]> {
  if (!pending) {
    pending = (async () => {
      await WhisperAPI.init();
      return WhisperAPI.getAvailableModels();
    })().finally(() => { pending = undefined; });
  }
  return pending;
}
