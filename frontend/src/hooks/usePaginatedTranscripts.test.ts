// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { usePaginatedTranscripts } from './usePaginatedTranscripts';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
afterEach(cleanup);

it('ignores late metadata, errors and pages after a meeting switch', async () => {
  const pending: Array<{ command: string; id: string; resolve: (value: any) => void; reject: (value: any) => void }> = [];
  vi.mocked(invoke).mockImplementation((command, args: any) => new Promise((resolve, reject) => {
    pending.push({ command, id: args.meetingId, resolve, reject });
  }));
  const { result, rerender } = renderHook(({ id }) => usePaginatedTranscripts({ meetingId: id }), { initialProps: { id: 'a' } });
  rerender({ id: 'b' });
  await act(async () => {
    pending.find(p => p.id === 'a' && p.command === 'api_get_meeting_metadata')!.reject('old error');
    pending.find(p => p.id === 'a' && p.command === 'api_get_meeting_transcripts')!.resolve({ transcripts: [{ id: 'old' }], total_count: 1, has_more: true });
  });
  expect(result.current.isLoading).toBe(true);
  expect(result.current.error).toBeNull();
  expect(result.current.transcripts).toEqual([]);
  await act(async () => {
    pending.find(p => p.id === 'b' && p.command === 'api_get_meeting_metadata')!.resolve({ id: 'b' });
    pending.find(p => p.id === 'b' && p.command === 'api_get_meeting_transcripts')!.resolve({ transcripts: [], total_count: 0, has_more: false });
  });
  expect(result.current.metadata?.id).toBe('b');
  expect(result.current.isLoading).toBe(false);
});

it('ignores a previous batch when refetching the same meeting', async () => {
  const pending: Array<(value: any) => void> = [];
  vi.mocked(invoke).mockImplementation(() => new Promise(resolve => pending.push(resolve)));
  const { result } = renderHook(() => usePaginatedTranscripts({ meetingId: 'a' }));
  let refresh!: Promise<void>;
  act(() => { refresh = result.current.refetch(); });
  await act(async () => {
    pending[0]({ id: 'stale' });
    pending[1]({ transcripts: [], total_count: 999, has_more: true });
  });
  expect(result.current.metadata).toBeNull();
  expect(result.current.isLoading).toBe(true);
  await act(async () => {
    pending[2]({ id: 'a' });
    pending[3]({ transcripts: [], total_count: 0, has_more: false });
    await refresh;
  });
  expect(result.current.totalCount).toBe(0);
});
it('starts metadata and transcripts together without waiting for metadata', async () => {
  let resolveMetadata!: (value: any) => void;
  vi.mocked(invoke).mockImplementation((command) => {
    if (command === 'api_get_meeting_metadata') return new Promise(resolve => { resolveMetadata = resolve; });
    return Promise.resolve({ transcripts: [], has_more: false, total_count: 0 });
  });
  const { result } = renderHook(() => usePaginatedTranscripts({ meetingId: 'a' }));
  expect(invoke).toHaveBeenCalledWith('api_get_meeting_transcripts', { meetingId: 'a', limit: 100, offset: 0 });
  await act(async () => { resolveMetadata({ id: 'a' }); });
  await waitFor(() => expect(result.current.isLoading).toBe(false));
});
