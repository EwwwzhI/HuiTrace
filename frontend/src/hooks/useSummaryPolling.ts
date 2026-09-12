import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Poll = { timer: ReturnType<typeof setInterval>; busy: boolean; count: number; started: number };

export function useSummaryPolling() {
  const polls = useRef(new Map<string, Poll>());
  const [activeSummaryPolls, setActiveSummaryPolls] = useState(new Map<string, ReturnType<typeof setInterval>>());
  const publish = useCallback(() => {
    setActiveSummaryPolls(new Map([...polls.current].map(([id, poll]) => [id, poll.timer])));
  }, []);
  const stopSummaryPolling = useCallback((meetingId: string) => {
    const poll = polls.current.get(meetingId);
    if (!poll) return;
    clearInterval(poll.timer);
    polls.current.delete(meetingId);
    publish();
  }, [publish]);
  const startSummaryPolling = useCallback((meetingId: string, _processId: string, onUpdate: (result: any) => void) => {
    stopSummaryPolling(meetingId);
    const poll: Poll = { timer: undefined!, busy: false, count: 0, started: Date.now() };
    const current = () => polls.current.get(meetingId) === poll;
    poll.timer = setInterval(async () => {
      if (!current()) return;
      if (Date.now() - poll.started >= 1000000) {
        stopSummaryPolling(meetingId);
        onUpdate({ status: 'error', error: 'Summary generation timed out. Please try again or check your model configuration.' });
        return;
      }
      if (poll.busy) return;
      poll.busy = true;
      poll.count++;
      try {
        const result = await invoke<any>('api_get_summary', { meetingId });
        if (!current()) return;
        if (['completed', 'error', 'failed', 'cancelled'].includes(result.status) || (result.status === 'idle' && poll.count > 1)) {
          stopSummaryPolling(meetingId);
        }
        onUpdate(result);
      } catch (error) {
        if (!current()) return;
        stopSummaryPolling(meetingId);
        onUpdate({ status: 'error', error: error instanceof Error ? error.message : String(error) });
      } finally {
        poll.busy = false;
      }
    }, 5000);
    polls.current.set(meetingId, poll);
    publish();
  }, [publish, stopSummaryPolling]);
  useEffect(() => {
    const registry = polls.current;
    return () => {
      registry.forEach(poll => clearInterval(poll.timer));
      registry.clear();
    };
  }, []);
  return { activeSummaryPolls, startSummaryPolling, stopSummaryPolling };
}
