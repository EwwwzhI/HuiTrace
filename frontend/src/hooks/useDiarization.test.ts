// @vitest-environment jsdom
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { useDiarization } = await import('./useDiarization');

afterEach(() => {
  invoke.mockReset();
});

const TURNS = [{ start_ms: 0, end_ms: 5_000, speaker_label: 'Speaker 1', confidence: null }];

describe('useDiarization', () => {
  /**
   * The field really is `diarized_at` next to a camelCase `status` — serde's
   * `rename_all` renames enum variants and leaves struct-variant fields alone.
   * Reading `diarizedAt` here would put `undefined` on screen and still
   * typecheck, so the snake_case name is pinned by a test on this side too.
   */
  it('reads the wire shape the Rust side actually sends', async () => {
    invoke.mockImplementation((cmd: string) =>
      cmd === 'api_diarization_availability'
        ? Promise.resolve({ status: 'done', diarized_at: '2026-08-09T10:00:00Z', turns: 1 })
        : cmd === 'api_get_speaker_turns'
          ? Promise.resolve(TURNS)
          : Promise.resolve([])
    );
    const { result } = renderHook(() => useDiarization('m1'));
    await waitFor(() => expect(result.current.state?.kind).toBe('done'));
    expect(result.current.state).toEqual({
      kind: 'done',
      diarizedAt: '2026-08-09T10:00:00Z',
      turns: TURNS,
    });
    expect(invoke.mock.calls.map((call) => call[0])).toContain('api_get_short_turn_events');
  });

  /** One query, not two, for a meeting that was never diarized. */
  it('does not fetch turns for a meeting that has none', async () => {
    invoke.mockResolvedValue({ status: 'ready' });
    const { result } = renderHook(() => useDiarization('m1'));
    await waitFor(() => expect(result.current.state?.kind).toBe('ready'));
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(['api_diarization_availability']);
    expect(result.current.turns).toEqual([]);
  });

  /**
   * A failure to ASK is not a state of the pass. Rendering it as one would tell
   * someone their recording has no audio when the truth is we could not find
   * out.
   */
  it('reports a failed query as an error, never as a state', async () => {
    invoke.mockRejectedValue('database is locked');
    const { result } = renderHook(() => useDiarization('m1'));
    await waitFor(() => expect(result.current.error).toContain('database is locked'));
    expect(result.current.state).toBeNull();
  });

  /** Post-hoc only: no meeting id (e.g. while recording) means no queries. */
  it('asks nothing when there is no meeting to ask about', async () => {
    const { result } = renderHook(() => useDiarization(undefined));
    await waitFor(() => expect(result.current.state).toBeNull());
    expect(invoke).not.toHaveBeenCalled();
  });

  /**
   * Zero turns is a real outcome, and only the stored stamp separates it from
   * "never ran" — so the state is re-read from the database rather than
   * inferred from the count the command returned.
   */
  it('re-reads after a pass instead of trusting the returned count', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'api_diarization_availability') {
        return Promise.resolve(
          invoke.mock.calls.filter((c) => c[0] === 'api_diarize_meeting').length === 0
            ? { status: 'ready' }
            : { status: 'done', diarized_at: '2026-08-09T11:00:00Z', turns: 0 }
        );
      }
      if (cmd === 'api_diarize_meeting') return Promise.resolve(0);
      return Promise.resolve([]);
    });

    const { result } = renderHook(() => useDiarization('m1'));
    await waitFor(() => expect(result.current.state?.kind).toBe('ready'));
    await act(async () => {
      await result.current.run();
    });
    expect(result.current.state).toEqual({
      kind: 'done',
      diarizedAt: '2026-08-09T11:00:00Z',
      turns: [],
    });
  });

  it('surfaces a failed pass without wiping what is already known', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'api_diarization_availability') return Promise.resolve({ status: 'ready' });
      if (cmd === 'api_diarize_meeting') return Promise.reject('diarize-helper not found');
      return Promise.resolve([]);
    });
    const { result } = renderHook(() => useDiarization('m1'));
    await waitFor(() => expect(result.current.state?.kind).toBe('ready'));
    await act(async () => {
      await result.current.run();
    });
    expect(result.current.error).toContain('diarize-helper not found');
    expect(result.current.state?.kind).toBe('ready');
    expect(result.current.busy).toBe(false);
  });

  it('uses the restore command boundary for an automatic short-event assignment', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'api_diarization_availability') return Promise.resolve({ status: 'done', diarized_at: 'now', turns: 1 });
      if (cmd === 'api_get_speaker_turns') return Promise.resolve(TURNS);
      return Promise.resolve([]);
    });
    const { result } = renderHook(() => useDiarization('m1'));
    await waitFor(() => expect(result.current.state?.kind).toBe('done'));
    invoke.mockClear();
    await act(async () => {
      await result.current.assignShortTurnEventSpeaker('event-1', null);
    });
    expect(invoke).toHaveBeenCalledWith('api_restore_short_turn_event_speaker', {
      meetingId: 'm1',
      eventId: 'event-1',
    });
  });
});
