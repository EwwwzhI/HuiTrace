// @vitest-environment jsdom
import { act, renderHook, waitFor, cleanup } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useImportAudio } from './useImportAudio';
const mocks = vi.hoisted(() => ({ listeners: new Map<string, (e: any) => void>(), invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name, callback) => { mocks.listeners.set(name, callback); return () => mocks.listeners.delete(name); }) }));
vi.mock('@/lib/analytics', () => ({ default: { track: vi.fn(async () => {}), trackError: vi.fn(async () => {}) } }));
vi.mock('@/lib/summary-language-preferences', () => ({ applyPinnedSummaryLanguageToMeeting: vi.fn(async () => {}) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
it('receives completion and hands the imported note to the caller', async () => {
  const done = vi.fn(); const { result } = renderHook(() => useImportAudio({ onComplete: done }));
  await waitFor(() => expect(mocks.listeners.has('import-complete')).toBe(true));
  await act(async () => { await result.current.startImport('selected.wav', 'Interview', null, 'model', 'whisper'); });
  expect(mocks.invoke).toHaveBeenCalledWith('start_import_audio_command', expect.objectContaining({ sourcePath: 'selected.wav', title: 'Interview' }));
  const payload = { meeting_id: 'note', title: 'Interview', segments_count: 3, duration_seconds: 12 };
  await act(async () => { await mocks.listeners.get('import-complete')!({ payload }); });
  expect(done).toHaveBeenCalledWith(payload); expect(result.current.status).toBe('complete');
});
it('does not report cancellation success before native cleanup finishes', async () => {
  let finish!: () => void;
  mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { finish = () => resolve(null); }));
  const { result } = renderHook(() => useImportAudio());
  let pending!: Promise<void>;
  await act(async () => { pending = result.current.cancelImport(); });
  let settled = false; void pending.then(() => { settled = true; });
  expect(settled).toBe(false);
  await act(async () => { finish(); await pending; }); expect(settled).toBe(true);
});
