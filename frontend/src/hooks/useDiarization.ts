'use client';

/**
 * The speaker-separation pass for one meeting: what state it is in, and the two
 * things a person can ask for.
 *
 * The pass is post-hoc by design (ADR-0034) -- it never runs during capture, so
 * nothing here can disturb a recording in progress. It is also entirely
 * on-device apart from one explicit model download, which is why fetching the
 * models is a separate action a person has to choose rather than something that
 * happens on first view.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type { DiarizationAvailability, SpeakerTurn } from '@/lib/speakerTurns';
import type { TalkTimeState } from '@/components/report/SpeakerTurns';

/**
 * Availability plus the turns themselves.
 *
 * `availability` alone reports only how many turns exist, because that is all
 * the four-state decision needs; the rows are fetched separately so a meeting
 * that was never diarized costs one query instead of two.
 */
export function useDiarization(meetingId: string | undefined) {
  const [state, setState] = useState<TalkTimeState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!meetingId) {
      setState(null);
      return;
    }
    try {
      const availability = await invoke<DiarizationAvailability>('api_diarization_availability', {
        meetingId,
      });
      if (availability.status !== 'done') {
        setState('error' in availability
          ? { kind: availability.status, error: availability.error }
          : { kind: availability.status });
        return;
      }
      // Only now are the rows worth fetching.
      const turns = await invoke<SpeakerTurn[]>('api_get_speaker_turns', { meetingId });
      setState({ kind: 'done', diarizedAt: availability.diarized_at, turns });
    } catch (e) {
      // A failure to ASK is not a state of the pass, so it must not be rendered
      // as one -- reporting it as `noAudio` would tell the user their recording
      // has no audio when the truth is that we could not find out.
      setError(String(e));
      setState(null);
    }
  }, [meetingId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!meetingId) return;
    let unlisten: (() => void) | undefined;
    void listen<{ meeting_id: string; status: string; error?: string }>('diarization-status-changed', (event) => {
      if (event.payload.meeting_id !== meetingId) return;
      if (event.payload.status === 'failed' && event.payload.error) setError(event.payload.error);
      void refresh();
    }).then((dispose) => { unlisten = dispose; }).catch(() => {
      // The web/test runtime has no Tauri event bridge. Availability remains
      // readable there; the desktop runtime installs this listener normally.
    });
    return () => unlisten?.();
  }, [meetingId, refresh]);

  const downloadModels = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke('api_diarization_download_models');
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  const run = useCallback(async () => {
    if (!meetingId) return;
    setBusy(true);
    setError(null);
    try {
      await invoke<'scheduled' | 'alreadyRunning'>('api_diarize_meeting', { meetingId });
      // Deliberately re-read rather than trusting the returned count: zero turns
      // is a real outcome, and the stamp that distinguishes it from "never ran"
      // only exists in the database.
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [meetingId, refresh]);

  const turns: SpeakerTurn[] = state?.kind === 'done' ? state.turns : [];

  const renameSpeaker = useCallback(async (speakerKey: string, displayName: string) => {
    if (!meetingId || !displayName.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await invoke('api_rename_meeting_speaker', { meetingId, speakerKey, displayName: displayName.trim() });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [meetingId, refresh]);

  const assignTranscriptSpeaker = useCallback(async (transcriptId: string, speakerKey: string | null) => {
    if (!meetingId) return;
    setError(null);
    try {
      await invoke(speakerKey === null ? 'api_restore_transcript_speaker_assignment' : 'api_assign_transcript_speaker',
        speakerKey === null ? { meetingId, transcriptId } : { meetingId, transcriptId, speakerKey });
    } catch (e) { setError(String(e)); throw e; }
  }, [meetingId]);

  return { state, turns, busy, error, run, downloadModels, refresh, renameSpeaker, assignTranscriptSpeaker };
}
