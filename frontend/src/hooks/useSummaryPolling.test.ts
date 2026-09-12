// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useSummaryPolling } from './useSummaryPolling';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
beforeEach(() => { vi.useFakeTimers(); vi.clearAllMocks(); });
afterEach(() => { cleanup(); vi.useRealTimers(); });

it('keeps other polls running and keeps callbacks stable across registry updates', async () => {
  vi.mocked(invoke).mockResolvedValue({ status: 'processing' });
  const { result, unmount } = renderHook(useSummaryPolling);
  const stop = result.current.stopSummaryPolling;
  const a = vi.fn(), b = vi.fn();
  act(() => { result.current.startSummaryPolling('a', '1', a); });
  act(() => { result.current.startSummaryPolling('b', '2', b); });
  expect(result.current.stopSummaryPolling).toBe(stop);
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(a).toHaveBeenCalledTimes(1);
  expect(b).toHaveBeenCalledTimes(1);
  act(() => { result.current.stopSummaryPolling('a'); });
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(a).toHaveBeenCalledTimes(1);
  expect(b).toHaveBeenCalledTimes(2);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});

it('prevents overlapping requests and ignores results from a replaced poll', async () => {
  let finish!: (value: any) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  const { result } = renderHook(useSummaryPolling);
  const old = vi.fn(), current = vi.fn();
  act(() => { result.current.startSummaryPolling('a', 'old', old); });
  await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
  expect(invoke).toHaveBeenCalledTimes(1);
  act(() => { result.current.startSummaryPolling('a', 'new', current); });
  await act(async () => { finish({ status: 'completed' }); });
  expect(old).not.toHaveBeenCalled();
  expect(result.current.activeSummaryPolls.has('a')).toBe(true);
  vi.mocked(invoke).mockResolvedValue({ status: 'completed' });
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(current).toHaveBeenCalledOnce();
  expect(result.current.activeSummaryPolls.size).toBe(0);
});
