import { expect, it, vi } from 'vitest';
import { WhisperAPI } from '@/lib/whisper';
import { loadWhisperModels } from './whisperModelsService';

vi.mock('@/lib/whisper', () => ({ WhisperAPI: {
  init: vi.fn().mockResolvedValue(undefined),
  getAvailableModels: vi.fn(),
} }));

it('shares an active scan and scans again after completion', async () => {
  let finish!: (models: []) => void;
  vi.mocked(WhisperAPI.getAvailableModels).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  const first = loadWhisperModels();
  const second = loadWhisperModels();
  expect(second).toBe(first);
  await Promise.resolve();
  expect(WhisperAPI.getAvailableModels).toHaveBeenCalledTimes(1);
  finish([]);
  await first;
  vi.mocked(WhisperAPI.getAvailableModels).mockResolvedValueOnce([]);
  await loadWhisperModels();
  expect(WhisperAPI.getAvailableModels).toHaveBeenCalledTimes(2);
});

it('allows retry after a failed scan', async () => {
  vi.mocked(WhisperAPI.getAvailableModels).mockRejectedValueOnce('Disk unavailable');
  await expect(loadWhisperModels()).rejects.toBe('Disk unavailable');
  vi.mocked(WhisperAPI.getAvailableModels).mockResolvedValueOnce([]);
  await expect(loadWhisperModels()).resolves.toEqual([]);
});
