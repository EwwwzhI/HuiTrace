'use client';

import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import {
  ChangeEvent, KeyboardEvent as ReactKeyboardEvent, PointerEvent, SyntheticEvent, useCallback, useEffect,
  useMemo, useRef, useState,
} from 'react';
import { toast } from 'sonner';
import { Check, ChevronLeft, ChevronRight, CircleHelp, Download, Languages, Moon, Pause, Play, Redo2, Sun, Undo2 } from 'lucide-react';
import { useTheme } from 'next-themes';
import { isTauri } from '@/lib/isTauri';
import { WaveformTimeline } from '@/components/short-turn-annotation/WaveformTimeline';
import { AnnotationProjectSetup } from '@/components/short-turn-annotation/AnnotationProjectSetup';
import { durationBucket, pendingInWindow, reviewCompletion } from '@/lib/annotationIntegrity';
import { isShortTurnAnnotationEnabled } from '@/lib/shortTurnAnnotationFeature';
import { useUiTranslation } from '@/i18n/client';
import { useUiLanguage } from '@/i18n/UiLanguageProvider';

type AnnotationPass = 'blind' | 'review';
type WorkspacePanel = 'annotation' | 'qa' | 'dataset';
type Kind = 'short_speech' | 'backchannel' | 'noise' | 'non_speech_vocalization' | 'ordinary_speech_control';
type EventStatus = 'pending' | 'review_pending' | 'blind_confirmed' | 'reviewed';
type AnnotationEvent = {
  event_id: string; start_ms: number; end_ms: number; kind: Kind; speaker: string | null;
  overlap: boolean; speaker_handoff: boolean; embedded: boolean; annotation_uncertain: boolean;
  expected_materialized: boolean | null; notes: string; annotation_status: EventStatus;
};
type Draft = { schema_version: number; meeting_id: string; events: AnnotationEvent[] };
type Speaker = { key: string; description: string };
type Session = {
  schema_version: number; meeting_id: string; source_media_path: string; production_artifact_path: string;
  source_media_duration_ms?: number | null;
  speaker_map: Speaker[]; window_status: Record<string, string>;
  next_event_sequence: number; last_blind_window_id?: string | null; last_review_window_id?: string | null;
};
type Suggestion = { start_ms: number; end_ms: number; candidate_sources: string[]; vad_confidence: number | null; label: string };
type WindowRow = { window_id: string; meeting_id: string; source_start_ms: number; source_end_ms: number; audio_path: string; candidate_suggestions: Suggestion[] };
type ReviewEvidence = { transcripts: { start_ms: number; end_ms: number; text: string }[]; diarizer_turns: { start_ms: number; end_ms: number; speaker_key: string; overlap: boolean }[]; vad_events: { start_ms: number; end_ms: number; confidence: number | null }[] };
type Snapshot = { draft: Draft; session: Session; windows: WindowRow[]; mode: AnnotationPass; reviewEvidence: ReviewEvidence | null; initialized: boolean };
type QaReport = { errors: string[]; possibleDuplicates: { first_event_id: string; second_event_id: string; overlap_iou: number; center_distance_ms: number }[]; sourceDurationMs: number | null };
type CheckReport = { representative_data_gate: string; coverage: Record<string, number | Record<string, number> | string[]>; possible_duplicates: unknown[] };

const kindKeys: { value: Kind; label: string; help: string }[] = [
  { value: 'short_speech', label: 'Short speech', help: 'A brief but independently meaningful utterance, such as “okay”, “no”, or “I do not think so”.' },
  { value: 'backchannel', label: 'Backchannel', help: 'A short response that acknowledges or encourages the current speaker, such as “mm”, “right”, or “yes”.' },
  { value: 'noise', label: 'Environmental noise', help: 'Non-vocal sounds such as keyboards, chairs, taps, or doors.' },
  { value: 'non_speech_vocalization', label: 'Non-speech vocalization', help: 'Non-linguistic human sounds such as coughing, laughter, sighing, or throat clearing.' },
  { value: 'ordinary_speech_control', label: 'Ordinary speech control', help: 'Normal speech longer than the short-turn range, retained as an evaluation control.' },
];

const defaultDraft = (meeting = ''): Draft => ({ schema_version: 1, meeting_id: meeting, events: [] });
const defaultSession = (meeting = ''): Session => ({ schema_version: 1, meeting_id: meeting, source_media_path: '', production_artifact_path: '', speaker_map: [], window_status: {}, next_event_sequence: 1 });
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
const fmt = (value: number) => `${(value / 1000).toFixed(3)}s`;

function meetingIdFromLocation() {
  if (typeof window === 'undefined') return '';
  return new URLSearchParams(window.location.search).get('meetingId')?.trim() ?? '';
}

function mediaUrl(path: string) {
  if (!path) return '';
  return isTauri() ? convertFileSrc(path) : path;
}

function isTextEntry(target: EventTarget | null) {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** Deliberately unlinked development/evaluation surface.  Production users do not see it. */
export default function ShortTurnAnnotationPage() {
  const enabled = isShortTurnAnnotationEnabled();
  const { t, language } = useUiTranslation();
  const { setChoice } = useUiLanguage();
  const { resolvedTheme, setTheme } = useTheme();
  const [themeMounted, setThemeMounted] = useState(false);
  const [datasetDir, setDatasetDir] = useState('evaluation/short_turn_dataset/local');
  const [meetingId, setMeetingId] = useState(meetingIdFromLocation);
  const [annotationPass, setAnnotationPass] = useState<AnnotationPass>('blind');
  const [panel, setPanel] = useState<WorkspacePanel>('annotation');
  const [initialized, setInitialized] = useState(false);
  const [draft, setDraft] = useState<Draft>(defaultDraft());
  const [session, setSession] = useState<Session>(defaultSession());
  const [windows, setWindows] = useState<WindowRow[]>([]);
  const [windowIndex, setWindowIndex] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [history, setHistory] = useState<Draft[]>([]);
  const [future, setFuture] = useState<Draft[]>([]);
  const [editRevision, setEditRevision] = useState(0);
  const [savedRevision, setSavedRevision] = useState(0);
  const [savingRevision, setSavingRevision] = useState<number | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);
  const [qaRevision, setQaRevision] = useState<number | null>(null);
  const [qa, setQa] = useState<QaReport | null>(null);
  const [check, setCheck] = useState<CheckReport | null>(null);
  const [reviewEvidence, setReviewEvidence] = useState<ReviewEvidence | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentMs, setCurrentMs] = useState(0);
  const [loop, setLoop] = useState(false);
  const [dragStart, setDragStart] = useState<number | null>(null);
  const audioRef = useRef<HTMLAudioElement | HTMLVideoElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const loopRange = useRef<{ start: number; end: number } | null>(null);
  const draftRef = useRef(draft); const sessionRef = useRef(session); const editRevisionRef = useRef(0); const saveInFlightRef = useRef(false); const allocatorRef = useRef(1);
  draftRef.current = draft; sessionRef.current = session; editRevisionRef.current = editRevision;

  useEffect(() => setThemeMounted(true), []);

  const currentWindow = windows[windowIndex];
  const reviewProgress = reviewCompletion(windows, session.window_status);
  const selected = draft.events.find(event => event.event_id === selectedId) ?? null;
  const viewportStart = currentWindow?.source_start_ms ?? 0;
  const viewportEnd = currentWindow?.source_end_ms ?? 5000;
  const viewportDuration = Math.max(1, viewportEnd - viewportStart);
  const displayedEvents = useMemo(() => draft.events.filter(event => event.start_ms < viewportEnd && event.end_ms > viewportStart).sort((a, b) => a.start_ms - b.start_ms), [draft.events, viewportStart, viewportEnd]);
  const video = /\.(mp4|webm)$/i.test(session.source_media_path);
  const activeLanguage = language.startsWith('zh') ? 'zh-CN' : 'en';

  const markDirty = useCallback(() => { setEditRevision(value => { const next = value + 1; editRevisionRef.current = next; return next; }); setQa(null); setQaRevision(null); setCheck(null); }, []);
  const commit = useCallback((mutation: (current: Draft) => Draft) => {
    setDraft(current => { setHistory(history => [...history.slice(-49), current]); setFuture([]); markDirty(); return mutation(current); });
  }, [markDirty]);
  const updateSession = useCallback((mutation: (current: Session) => Session) => {
    setSession(current => mutation(current)); markDirty();
  }, [markDirty]);

  useEffect(() => {
    const invalid = windows.filter(window => {
      const status = session.window_status[window.window_id];
      return (status === 'reviewed_blind' || status === 'reviewed_second_pass')
        && pendingInWindow(draft.events, window, status === 'reviewed_second_pass') > 0;
    });
    if (!invalid.length) return;
    updateSession(current => {
      const statuses = { ...current.window_status };
      for (const window of invalid) {
        if (statuses[window.window_id] === 'reviewed_second_pass' && pendingInWindow(draft.events, window, false) === 0) statuses[window.window_id] = 'reviewed_blind';
        else delete statuses[window.window_id];
      }
      return { ...current, window_status: statuses };
    });
  }, [draft.events, session.window_status, updateSession, windows]);

  const load = useCallback(async (nextPass = annotationPass) => {
    if (!datasetDir.trim() || !meetingId.trim()) return toast.error(t('Dataset folder and Meeting ID are required'));
    try {
      const result = await invoke<Snapshot>('load_workspace', { request: { datasetDir, meetingId, mode: nextPass } });
      setDraft(result.draft); setSession(result.session); draftRef.current=result.draft; sessionRef.current=result.session; allocatorRef.current=result.session.next_event_sequence || 1; setWindows(result.windows); setReviewEvidence(result.reviewEvidence); setAnnotationPass(nextPass); setPanel('annotation'); setInitialized(result.initialized); const lastId = nextPass === 'blind' ? result.session.last_blind_window_id : result.session.last_review_window_id; const restored = result.windows.findIndex(row => row.window_id === lastId); setWindowIndex(restored >= 0 ? restored : 0); setSelectedId(null); setHistory([]); setFuture([]); setEditRevision(0); editRevisionRef.current=0; setSavedRevision(0); setSavingRevision(null); setSaveFailed(false); setQa(null); setQaRevision(null); setCheck(null);
    } catch (error) { toast.error(String(error)); }
  }, [datasetDir, meetingId, annotationPass, t]);

  const flushSave = useCallback(async () => {
    if (!isTauri() || !initialized || saveInFlightRef.current || editRevisionRef.current <= savedRevision) return;
    saveInFlightRef.current = true; const revision = editRevisionRef.current; setSavingRevision(revision); setSaveFailed(false);
    try { await invoke('save_workspace', { request: { datasetDir, draft: draftRef.current, session: sessionRef.current } }); setSavedRevision(value => Math.max(value, revision)); }
    catch (error) { setSaveFailed(true); toast.error(`${t('Save failed')}: ${String(error)}`); }
    finally { saveInFlightRef.current = false; setSavingRevision(null); if (editRevisionRef.current > revision) window.setTimeout(() => void flushSave(), 0); }
  }, [datasetDir, initialized, savedRevision, t]);
  useEffect(() => { if (editRevision <= savedRevision || !initialized) return; const timer = window.setTimeout(() => void flushSave(), 400); return () => window.clearTimeout(timer); }, [editRevision, savedRevision, initialized, flushSave]);

  const seek = useCallback((ms: number) => { const media = audioRef.current; if (!media) return; media.currentTime = Math.max(0, ms / 1000); setCurrentMs(ms); }, []);
  const togglePlay = useCallback(async () => { const media = audioRef.current; if (!media) return; if (media.paused) { await media.play(); } else { media.pause(); } }, []);
  const playSelection = useCallback((padding = 300) => {
    if (!selected) return;
    const start = Math.max(0, selected.start_ms - padding); const end = selected.end_ms + padding;
    loopRange.current = { start, end }; seek(start); audioRef.current?.play();
  }, [selected, seek]);
  const onTimeUpdate = useCallback(() => {
    const media = audioRef.current; if (!media) return;
    const ms = Math.round(media.currentTime * 1000); setCurrentMs(ms);
    if (loopRange.current && ms >= loopRange.current.end) {
      if (loop) { media.currentTime = loopRange.current.start / 1000; } else { media.pause(); loopRange.current = null; }
    }
  }, [loop]);
  const onLoadedMetadata = useCallback((event: SyntheticEvent<HTMLMediaElement>) => {
    // React clears currentTarget after the handler returns. Capture the primitive
    // before scheduling the state updater so it never reads from a released event.
    const duration = event.currentTarget.duration;
    if (!Number.isFinite(duration) || duration < 0) return;
    const durationMs = Math.round(duration * 1000);
    updateSession(current => ({ ...current, source_media_duration_ms: durationMs }));
  }, [updateSession]);

  const createEvent = useCallback((start: number, end: number) => {
    const sequence = allocatorRef.current++; updateSession(current => ({ ...current, next_event_sequence: allocatorRef.current }));
    const event: AnnotationEvent = { event_id: `${meetingId}-event-${String(sequence).padStart(4, '0')}`, start_ms: Math.round(Math.min(start, end)), end_ms: Math.round(Math.max(start, end)), kind: 'short_speech', speaker: null, overlap: false, speaker_handoff: false, embedded: false, annotation_uncertain: false, expected_materialized: null, notes: '', annotation_status: annotationPass === 'blind' ? 'pending' : 'review_pending' };
    if (event.end_ms - event.start_ms < 30) return;
    commit(current => ({ ...current, events: [...current.events, event] })); setSelectedId(event.event_id);
  }, [annotationPass, commit, meetingId, updateSession]);
  const updateSelected = useCallback((patch: Partial<AnnotationEvent>) => { if (!selectedId) return; commit(current => ({ ...current, events: current.events.map(event => event.event_id === selectedId ? { ...event, ...patch } : event) })); }, [commit, selectedId]);
  const deleteSelected = useCallback(() => { if (!selectedId) return; commit(current => ({ ...current, events: current.events.filter(event => event.event_id !== selectedId) })); setSelectedId(null); }, [commit, selectedId]);
  const undo = useCallback(() => { setHistory(items => { const prior = items.at(-1); if (!prior) return items; setFuture(items2 => [draft, ...items2].slice(0, 50)); setDraft(prior); markDirty(); return items.slice(0, -1); }); }, [draft, markDirty]);
  const redo = useCallback(() => { setFuture(items => { const next = items[0]; if (!next) return items; setHistory(previous => [...previous, draft]); setDraft(next); markDirty(); return items.slice(1); }); }, [draft, markDirty]);

  const pointToMs = (event: PointerEvent<HTMLDivElement>) => {
    const rect = timelineRef.current?.getBoundingClientRect(); if (!rect) return viewportStart;
    return Math.round(viewportStart + clamp((event.clientX - rect.left) / rect.width, 0, 1) * viewportDuration);
  };
  const onTimelineDown = (event: PointerEvent<HTMLDivElement>) => { if ((event.target as HTMLElement).closest('[data-region]')) return; const start = pointToMs(event); setDragStart(start); (event.currentTarget as HTMLDivElement).setPointerCapture(event.pointerId); };
  const onTimelineUp = (event: PointerEvent<HTMLDivElement>) => { if (dragStart === null) { seek(pointToMs(event)); return; } const end = pointToMs(event); setDragStart(null); if (Math.abs(end - dragStart) < 30) seek(end); else createEvent(dragStart, end); };
  const dragEdge = (event: PointerEvent<HTMLButtonElement>, edge: 'start' | 'end', item: AnnotationEvent) => {
    event.preventDefault(); event.stopPropagation(); const button = event.currentTarget; button.setPointerCapture(event.pointerId);
    const move = (moveEvent: globalThis.PointerEvent) => { const rect = timelineRef.current?.getBoundingClientRect(); if (!rect) return; const value = Math.round(viewportStart + clamp((moveEvent.clientX - rect.left) / rect.width, 0, 1) * viewportDuration); commit(current => ({ ...current, events: current.events.map(candidate => candidate.event_id !== item.event_id ? candidate : edge === 'start' ? { ...candidate, start_ms: Math.min(value, candidate.end_ms - 1) } : { ...candidate, end_ms: Math.max(value, candidate.start_ms + 1) }) })); };
    const end = () => { button.removeEventListener('pointermove', move); button.removeEventListener('pointerup', end); };
    button.addEventListener('pointermove', move); button.addEventListener('pointerup', end);
  };

  const runQa = useCallback(async () => { try { const result = await invoke<QaReport>('qa_workspace_command', { datasetDir, draft, session }); setQa(result); setQaRevision(editRevision); setPanel('qa'); } catch (error) { toast.error(String(error)); } }, [datasetDir, draft, editRevision, session]);
  const exportManifest = useCallback(async () => { try { const result = await invoke<CheckReport>('export_manifest_command', { datasetDir, draft, session }); setCheck(result); toast.success(t('Benchmark manifest exported and checked locally')); } catch (error) { toast.error(String(error)); } }, [datasetDir, draft, session, t]);
  const goToWindow = useCallback((index: number) => { const next = clamp(index, 0, Math.max(0, windows.length - 1)); setWindowIndex(next); const row=windows[next]; if (row) { seek(row.source_start_ms); updateSession(current => ({ ...current, [annotationPass === 'blind' ? 'last_blind_window_id' : 'last_review_window_id']: row.window_id })); } }, [annotationPass, seek, updateSession, windows]);
  const completeWindow = useCallback(() => { if (!currentWindow) return; const pending = pendingInWindow(draft.events, currentWindow, annotationPass === 'review'); if (pending) { toast.error(t('This window still contains {{count}} pending annotations. Confirm all event labels before completing the window.', { count: pending })); return; } const status = annotationPass === 'blind' ? 'reviewed_blind' : 'reviewed_second_pass'; updateSession(current => ({ ...current, window_status: { ...current.window_status, [currentWindow.window_id]: status } })); const next = windows.findIndex((row, index) => index > windowIndex && session.window_status[row.window_id] !== status); if (next >= 0) goToWindow(next); }, [annotationPass, currentWindow, draft.events, goToWindow, session.window_status, t, updateSession, windowIndex, windows]);
  const initializeProject = useCallback(async (sourceMediaPath: string, productionArtifactPath: string) => {
    if (!datasetDir.trim() || !meetingId.trim()) {
      toast.error(t('Dataset folder and Meeting ID are required'));
      return false;
    }
    try {
      const result = await invoke<Snapshot>('initialize_annotation_project', { request: { datasetDir, meetingId, sourceMediaPath, productionArtifactPath } });
      setDraft(result.draft); setSession(result.session); draftRef.current=result.draft; sessionRef.current=result.session; allocatorRef.current=result.session.next_event_sequence; setWindows(result.windows); setReviewEvidence(null); setInitialized(true); setAnnotationPass('blind'); setPanel('annotation'); setWindowIndex(0); setEditRevision(0); setSavedRevision(0); toast.success(t('Annotation project initialized'));
      return true;
    } catch (error) {
      console.error('initialize_annotation_project failed', error);
      toast.error(`${t('Initialize failed')}: ${String(error)}`);
      return false;
    }
  }, [datasetDir, meetingId, t]);

  useEffect(() => {
    const handler = (event: globalThis.KeyboardEvent) => {
      if (isTextEntry(event.target)) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); event.shiftKey ? redo() : undo(); return; }
      if (event.key === ' ') { event.preventDefault(); togglePlay(); return; }
      if (event.key === 'ArrowLeft') { event.preventDefault(); seek(currentMs - (event.shiftKey ? 500 : 100)); return; }
      if (event.key === 'ArrowRight') { event.preventDefault(); seek(currentMs + (event.shiftKey ? 500 : 100)); return; }
      if (event.key.toLowerCase() === 'j') { seek(currentMs - 1000); return; }
      if (event.key.toLowerCase() === 'l') { seek(currentMs + 1000); return; }
      if (event.key === 'Delete') { deleteSelected(); return; }
      if (event.key === 'Enter') { completeWindow(); return; }
      const kindByKey: Record<string, Kind> = { b: 'backchannel', s: 'short_speech', n: 'noise', v: 'non_speech_vocalization', c: 'ordinary_speech_control' };
      if (kindByKey[event.key.toLowerCase()] && selected) { updateSelected({ kind: kindByKey[event.key.toLowerCase()] }); return; }
      if (selected && /^[1-9]$/.test(event.key)) { const speaker = session.speaker_map[Number(event.key) - 1]; if (speaker) updateSelected({ speaker: speaker.key }); return; }
      if (selected && ({ o: 'overlap', h: 'speaker_handoff', e: 'embedded', u: 'annotation_uncertain' } as const)[event.key.toLowerCase() as 'o' | 'h' | 'e' | 'u']) { const flag = ({ o: 'overlap', h: 'speaker_handoff', e: 'embedded', u: 'annotation_uncertain' } as const)[event.key.toLowerCase() as 'o' | 'h' | 'e' | 'u']; updateSelected({ [flag]: !selected[flag] }); }
    };
    window.addEventListener('keydown', handler); return () => window.removeEventListener('keydown', handler);
  }, [completeWindow, currentMs, deleteSelected, redo, seek, selected, session.speaker_map, togglePlay, undo, updateSelected]);

  const saveStatus = saveFailed
    ? t('Save failed')
    : savingRevision !== null
      ? t('Saving revision…', { revision: savingRevision })
      : editRevision === savedRevision ? t('Saved locally') : t('Unsaved');
  const panelLabel = panel === 'annotation' ? t('Annotation') : panel === 'qa' ? t('Quality assurance') : t('Dataset');

  if (!enabled) return <main className="min-h-screen bg-background p-10 text-foreground"><h1 className="text-xl font-semibold">{t('Development evaluation route is disabled')}</h1><p className="mt-2 text-muted-foreground">{t('Enable the Short-Turn Annotation feature flag in a development or evaluation build.')}</p></main>;

  return <main data-testid="annotation-workspace" className="custom-scrollbar h-full min-h-0 overflow-y-auto overscroll-contain bg-background p-4 text-foreground">
    <header className="mb-3 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-card px-4 py-3 shadow-sm">
      <div>
        <p className="text-[11px] font-bold tracking-[0.18em] text-primary">HuiTrace / Phase 2D.2</p>
        <h1 className="font-heading text-lg font-semibold">{t('Short-Turn Annotation Workspace')}</h1>
        <p className="mt-0.5 text-xs text-muted-foreground">{t('Build, review, and export Ground Truth for short-turn speaker diarization evaluation.')}</p>
      </div>
      <div className="flex flex-wrap items-center justify-end gap-2 text-sm">
        <div role="radiogroup" aria-label={t('Language')} className="inline-flex rounded-lg border border-border bg-muted p-0.5">
          <button type="button" role="radio" aria-checked={activeLanguage === 'zh-CN'} onClick={() => setChoice('zh-CN')} className={`inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${activeLanguage === 'zh-CN' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:bg-background hover:text-foreground'}`}><Languages size={13} aria-hidden="true" />中文{activeLanguage === 'zh-CN' && <Check size={12} aria-hidden="true" />}</button>
          <button type="button" role="radio" aria-checked={activeLanguage === 'en'} onClick={() => setChoice('en')} className={`inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${activeLanguage === 'en' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:bg-background hover:text-foreground'}`}>English{activeLanguage === 'en' && <Check size={12} aria-hidden="true" />}</button>
        </div>
        <div role="radiogroup" aria-label={t('Theme')} className="inline-flex rounded-lg border border-border bg-muted p-0.5">
          <button type="button" role="radio" aria-checked={themeMounted && resolvedTheme === 'light'} onClick={() => setTheme('light')} className={`inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs ${themeMounted && resolvedTheme === 'light' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}><Sun size={13} aria-hidden="true" />{t('Light')}</button>
          <button type="button" role="radio" aria-checked={themeMounted && resolvedTheme === 'dark'} onClick={() => setTheme('dark')} className={`inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs ${themeMounted && resolvedTheme === 'dark' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}><Moon size={13} aria-hidden="true" />{t('Dark')}</button>
        </div>
        <span className={`rounded px-2 py-1 font-bold tracking-wide ${annotationPass === 'blind' ? 'bg-warning text-warning-foreground' : 'bg-violet-100 text-violet-900 dark:bg-violet-900/50 dark:text-violet-100'}`}>{annotationPass === 'blind' ? t('Blind annotation') : t('Review annotation')}</span>
        <span className="rounded border border-border px-2 py-1 text-xs">{panelLabel}</span>
        <span className="font-mono text-muted-foreground">{meetingId || t('No meeting')}</span>
        <span className="font-mono text-primary">{fmt(currentMs)}</span>
        <span aria-live="polite" className={saveFailed ? 'text-destructive' : editRevision === savedRevision && savingRevision === null ? 'text-success' : 'text-warning'}>{saveStatus}</span>
      </div>
    </header>
    {!initialized ? <>
      <AnnotationProjectSetup datasetRoot={datasetDir} setDatasetRoot={setDatasetDir} meetingId={meetingId} setMeetingId={setMeetingId} initialize={initializeProject} />
      <details className="mx-auto mt-3 max-w-3xl rounded-xl border border-border bg-card">
        <summary className="cursor-pointer px-4 py-3 text-sm font-semibold">{t('Open existing annotation project')}</summary>
        <section className="grid gap-2 border-t border-border p-3 md:grid-cols-[minmax(220px,1.3fr)_minmax(180px,1fr)_auto_auto]">
          <label className="text-xs text-muted-foreground">{t('Dataset Root')}<input value={datasetDir} onChange={event => setDatasetDir(event.target.value)} className="mt-1 w-full rounded border border-input bg-background px-2 py-1.5 font-mono text-sm text-foreground" /></label>
          <label className="text-xs text-muted-foreground">{t('Existing Meeting ID')}<input value={meetingId} onChange={event => setMeetingId(event.target.value)} className="mt-1 w-full rounded border border-input bg-background px-2 py-1.5 font-mono text-sm text-foreground" /></label>
          <div className="flex items-end gap-2"><button onClick={() => load('blind')} className="whitespace-nowrap rounded bg-primary px-3 py-2 text-sm font-semibold text-primary-foreground">{t('Open blind pass')}</button><button onClick={() => load('review')} className="whitespace-nowrap rounded border border-border bg-background px-3 py-2 text-sm text-foreground hover:bg-accent">{t('Review')}</button></div>
          <p className="self-end text-xs text-muted-foreground">{t('Local files only · no telemetry · no upload')}</p>
        </section>
      </details>
    </> : <>
      <section className="mb-3 grid gap-2 rounded-xl border border-border bg-card/70 p-3 md:grid-cols-[minmax(220px,1.3fr)_minmax(180px,1fr)_auto_auto]">
        <label className="text-xs text-muted-foreground">{t('Dataset Root')}<input readOnly value={datasetDir} className="mt-1 w-full rounded border border-input bg-muted/50 px-2 py-1.5 font-mono text-sm text-foreground" /></label>
        <label className="text-xs text-muted-foreground">{t('Meeting ID')}<input readOnly value={meetingId} className="mt-1 w-full rounded border border-input bg-muted/50 px-2 py-1.5 font-mono text-sm text-foreground" /></label>
        <div className="flex items-end gap-2"><button onClick={() => load('blind')} className="whitespace-nowrap rounded bg-primary px-3 py-2 text-sm font-semibold text-primary-foreground">{t('Open blind pass')}</button><button onClick={() => load('review')} className="whitespace-nowrap rounded border border-border bg-background px-3 py-2 text-sm text-foreground hover:bg-accent">{t('Review')}</button></div>
        <p className="self-end text-xs text-muted-foreground">{t('Local files only · no telemetry · no upload')}</p>
      </section>
      <section className="grid min-h-[560px] gap-3 xl:grid-cols-[minmax(280px,1.1fr)_minmax(300px,1fr)_minmax(300px,1fr)]">
        <aside className="rounded-xl border border-border bg-card p-3">
          <p className="mb-2 text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Source media')}</p>
          {video ? <video ref={audioRef as React.RefObject<HTMLVideoElement>} src={mediaUrl(session.source_media_path)} controls onLoadedMetadata={onLoadedMetadata} onTimeUpdate={onTimeUpdate} onPlay={() => setIsPlaying(true)} onPause={() => setIsPlaying(false)} className="aspect-video w-full rounded bg-black" /> : <div className="rounded bg-muted p-5"><audio ref={audioRef as React.RefObject<HTMLAudioElement>} src={mediaUrl(session.source_media_path)} controls onLoadedMetadata={onLoadedMetadata} onTimeUpdate={onTimeUpdate} onPlay={() => setIsPlaying(true)} onPause={() => setIsPlaying(false)} className="w-full" /><p className="mt-4 text-sm text-muted-foreground">{t('Audio-only source. Speaker map descriptions remain local-only.')}</p></div>}
          <div className="mt-3 flex flex-wrap gap-2"><button onClick={togglePlay} className="inline-flex items-center gap-1 rounded bg-secondary px-3 py-2 text-sm text-secondary-foreground">{isPlaying ? <Pause size={16} /> : <Play size={16} />}{isPlaying ? t('Pause') : t('Play')}</button>{[250, 500, 1000].map(padding => <button key={padding} onClick={() => playSelection(padding)} disabled={!selected} className="rounded border border-border bg-background px-2 py-1 text-xs hover:bg-accent disabled:opacity-40">{t('Event')} ±{padding} ms</button>)}<button onClick={() => setLoop(value => !value)} disabled={!selected} aria-pressed={loop} className={`rounded border px-2 py-1 text-xs disabled:opacity-40 ${loop ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-background'}`}>{t('Loop')}</button></div>
          <div className="mt-5 border-t border-border pt-3"><p className="mb-2 text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Speaker map — local only')}</p>{session.speaker_map.map((speaker, index) => <div className="mb-2 grid grid-cols-[8rem_1fr] gap-2" key={speaker.key}><span className="rounded bg-muted p-1 text-center font-mono text-xs">{speaker.key}</span><input value={speaker.description} placeholder={t('Local description')} aria-label={t('Speaker local description', { number: index + 1 })} onChange={event => updateSession(current => ({ ...current, speaker_map: current.speaker_map.map((item, itemIndex) => itemIndex === index ? { ...item, description: event.target.value } : item) }))} className="min-w-0 rounded border border-input bg-background px-2 text-sm text-foreground" /></div>)}<button onClick={() => updateSession(current => ({ ...current, speaker_map: [...current.speaker_map, { key: `gt_speaker_${String(current.speaker_map.length + 1).padStart(2, '0')}`, description: '' }] }))} className="text-sm text-primary underline">{t('Add speaker')}</button></div>
        </aside>
        <section className="rounded-xl border border-border bg-card p-3"><div className="mb-3 flex items-center justify-between"><div><p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Canonical events')}</p><p className="text-sm text-muted-foreground">{t('Window')} {windows.length ? `${windowIndex + 1} / ${windows.length}` : '—'} · {t('Current time range')} {fmt(viewportStart)}–{fmt(viewportEnd)}</p></div><div className="flex gap-1"><button aria-label={t('Previous window')} onClick={() => goToWindow(windowIndex - 1)} disabled={!windowIndex} className="rounded p-2 hover:bg-accent disabled:opacity-30"><ChevronLeft /></button><button aria-label={t('Next window')} onClick={() => goToWindow(windowIndex + 1)} disabled={windowIndex >= windows.length - 1} className="rounded p-2 hover:bg-accent disabled:opacity-30"><ChevronRight /></button></div></div>
          <div className="space-y-2 overflow-y-auto pr-1 xl:max-h-[490px]">{displayedEvents.length === 0 ? <p className="rounded border border-dashed border-border p-5 text-sm text-muted-foreground">{t('Drag a region in the source timeline to create one canonical event.')}</p> : displayedEvents.map(event => <button key={event.event_id} onClick={() => setSelectedId(event.event_id)} className={`w-full rounded-lg border p-3 text-left transition ${selectedId === event.event_id ? 'border-primary bg-primary/10' : 'border-border bg-background hover:border-primary/50'}`}><div className="flex justify-between gap-2"><span className="font-mono text-xs text-muted-foreground">{event.event_id}</span><span className="rounded bg-muted px-1.5 text-xs">{durationBucket(event.end_ms - event.start_ms)}</span></div><p className="mt-1 font-medium">{t(kindKeys.find(kind => kind.value === event.kind)?.label ?? event.kind)} <span className="font-normal text-muted-foreground">· {event.speaker ?? t('No speaker')}</span></p><p className="mt-1 font-mono text-xs text-muted-foreground">{fmt(event.start_ms)}–{fmt(event.end_ms)} · {event.end_ms - event.start_ms} ms {event.overlap && `· ${t('Overlap')}`} {event.speaker_handoff && `· ${t('Speaker handoff')}`} {event.embedded && `· ${t('Embedded')}`} {event.annotation_uncertain && `· ${t('Annotation uncertain')}`}</p></button>)}</div>
          <div className="mt-3 flex flex-wrap gap-2 border-t border-border pt-3"><button onClick={undo} disabled={!history.length} className="inline-flex items-center gap-1 rounded border border-border bg-background px-2 py-1 text-xs hover:bg-accent disabled:opacity-30"><Undo2 size={14} />{t('Undo')}</button><button onClick={redo} disabled={!future.length} className="inline-flex items-center gap-1 rounded border border-border bg-background px-2 py-1 text-xs hover:bg-accent disabled:opacity-30"><Redo2 size={14} />{t('Redo')}</button><button onClick={completeWindow} className="rounded bg-success px-2 py-1 text-xs font-semibold text-success-foreground">{t('Confirm and complete this window')}</button></div>
        </section>
        <Inspector selected={selected} speakers={session.speaker_map} update={updateSelected} remove={deleteSelected} />
      </section>
      <WaveformTimeline media={audioRef.current} sourceUrl={mediaUrl(session.source_media_path)} events={draft.events} selectedId={selectedId} currentMs={currentMs} onSeek={seek} onCreate={createEvent} onSelect={setSelectedId} onBoundsChange={(id, start_ms, end_ms) => commit(current => ({ ...current, events: current.events.map(event => event.event_id === id ? { ...event, start_ms, end_ms } : event) }))} />
      <section className="mt-3 rounded-xl border border-border bg-card p-3"><div className="mb-2 flex flex-wrap items-center justify-between gap-2"><div><p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Multi-tier source timeline')}</p><p className="text-xs text-muted-foreground">{t('Ground Truth is shown separately from every review-only system evidence tier.')}</p></div>{annotationPass === 'review' && <span className="rounded border border-violet-300 bg-violet-100 px-2 py-1 text-xs font-bold tracking-wide text-violet-900 dark:border-violet-500/70 dark:bg-violet-900/40 dark:text-violet-100">{t('System suggestion · not Ground Truth')}</span>}</div>
        <div ref={timelineRef} role="slider" aria-label={t('Source timeline')} aria-valuemin={viewportStart} aria-valuemax={viewportEnd} aria-valuenow={currentMs} tabIndex={0} onPointerDown={onTimelineDown} onPointerUp={onTimelineUp} className="relative h-28 touch-none cursor-crosshair overflow-hidden rounded-lg border border-border bg-muted/60">
          <div className="absolute inset-x-0 bottom-0 flex h-14 items-end gap-[3px] px-1 opacity-45" aria-hidden="true">{Array.from({ length: 110 }, (_, index) => <span key={index} style={{ height: `${20 + ((index * 37) % 70)}%` }} className="flex-1 rounded-t bg-cyan-600 dark:bg-cyan-300" />)}</div>
          {displayedEvents.map(event => <Region key={event.event_id} event={event} selected={event.event_id === selectedId} start={viewportStart} duration={viewportDuration} onSelect={() => setSelectedId(event.event_id)} onEdge={dragEdge} />)}
          {annotationPass === 'review' && currentWindow?.candidate_suggestions.map((suggestion, index) => <button key={index} type="button" title={t('Accept suggestion as a pending editable annotation')} onClick={() => createEvent(suggestion.start_ms, suggestion.end_ms)} className="absolute top-5 h-5 border border-dashed border-violet-500 bg-violet-200/50 dark:border-violet-300/80 dark:bg-violet-400/15" style={{ left: `${clamp((suggestion.start_ms - viewportStart) / viewportDuration * 100, 0, 100)}%`, width: `${clamp((suggestion.end_ms - suggestion.start_ms) / viewportDuration * 100, 0.3, 100)}%` }}><span className="absolute -top-4 whitespace-nowrap text-[9px] font-bold text-violet-800 dark:text-violet-200">{t('System · accept as pending')}</span></button>)}
          <div className="absolute inset-y-0 w-px bg-amber-600 dark:bg-amber-300" style={{ left: `${clamp((currentMs - viewportStart) / viewportDuration * 100, 0, 100)}%` }} />
        </div>
        <TierRows mode={annotationPass} evidence={reviewEvidence} start={viewportStart} end={viewportEnd} />
      </section>
      <section className="mt-3 grid gap-3 rounded-xl border border-border bg-card p-3 lg:grid-cols-[1fr_auto]"><div><p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Quality assurance and benchmark export')}</p><p className="mt-1 text-sm text-muted-foreground">{t('Blind QA is structural only. Dataset coverage and representative gates remain hidden until Review.')}</p>{annotationPass === 'review' && <p className="mt-1 text-sm text-muted-foreground">{t('Review progress {{completed}} / {{total}}', { completed: reviewProgress.completed, total: reviewProgress.total })}{!reviewProgress.complete && <> — {t('Complete all Review windows before exporting the Benchmark Manifest.')}</>}</p>}{qa && <QaView qa={qa} />}{annotationPass === 'review' && check && <CheckView check={check} />}</div><div className="flex flex-wrap content-start gap-2"><button onClick={runQa} className="inline-flex items-center gap-1 rounded border border-warning bg-warning/10 px-3 py-2 text-sm text-warning"><CircleHelp size={16} />{t('Run QA')}</button>{annotationPass === 'review' && <button onClick={exportManifest} disabled={!reviewProgress.complete || !qa || qa.errors.length > 0 || qaRevision !== editRevision || savedRevision !== editRevision} className="inline-flex items-center gap-1 rounded bg-primary px-3 py-2 text-sm font-semibold text-primary-foreground disabled:opacity-40"><Download size={16} />{t('Export benchmark manifest')}</button>}</div></section>
    </>}
    <ShortcutHelp />
  </main>;
}

function Region({ event, selected, start, duration, onSelect, onEdge }: { event: AnnotationEvent; selected: boolean; start: number; duration: number; onSelect: () => void; onEdge: (event: PointerEvent<HTMLButtonElement>, edge: 'start' | 'end', item: AnnotationEvent) => void }) {
  const { t } = useUiTranslation();
  const left = clamp((event.start_ms - start) / duration * 100, 0, 100); const width = clamp((event.end_ms - event.start_ms) / duration * 100, 0.5, 100);
  return <div data-region onPointerDown={event => { event.stopPropagation(); onSelect(); }} className={`absolute top-12 h-8 rounded ${selected ? 'bg-cyan-600/75 ring-2 ring-cyan-800 dark:bg-cyan-300/70 dark:ring-cyan-100' : 'bg-cyan-500/50 dark:bg-cyan-400/45'}`} style={{ left: `${left}%`, width: `${width}%` }}><button aria-label={t('Adjust start of event', { eventId: event.event_id })} onPointerDown={pointer => onEdge(pointer, 'start', event)} className="absolute -left-1 top-0 h-full w-2 cursor-ew-resize rounded bg-cyan-800 dark:bg-cyan-100" /><span className="pointer-events-none px-1 text-[10px] font-bold text-white dark:text-cyan-950">{t(kindKeys.find(kind => kind.value === event.kind)?.label ?? event.kind)}</span><button aria-label={t('Adjust end of event', { eventId: event.event_id })} onPointerDown={pointer => onEdge(pointer, 'end', event)} className="absolute -right-1 top-0 h-full w-2 cursor-ew-resize rounded bg-cyan-800 dark:bg-cyan-100" /></div>;
}

function TierRows({ mode, evidence, start, end }: { mode: AnnotationPass; evidence: ReviewEvidence | null; start: number; end: number }) {
  const { t } = useUiTranslation();
  if (mode === 'review') {
    const duration = Math.max(1, end - start);
    const band = (name: string, rows: { start_ms: number; end_ms: number; label: string; title: string }[], color: string) => <div className="grid grid-cols-[9rem_1fr] gap-3 px-3 py-2"><span className="font-semibold text-foreground">{name}</span><div className="relative h-7 overflow-hidden rounded bg-muted">{rows.filter(row => row.start_ms < end && row.end_ms > start).map((row, index) => <span key={`${row.start_ms}-${row.end_ms}-${index}`} title={row.title} className={`absolute inset-y-1 overflow-hidden whitespace-nowrap rounded px-1 text-[9px] ${color}`} style={{ left: `${clamp((row.start_ms-start)/duration*100,0,100)}%`, width: `${clamp((row.end_ms-row.start_ms)/duration*100,.4,100)}%` }}>{row.label}</span>)}</div></div>;
    return <div className="mt-3 divide-y divide-border rounded border border-border text-xs">
      {band(t('ASR transcript'), (evidence?.transcripts ?? []).map(row => ({ ...row, label: row.text, title: `${fmt(row.start_ms)}–${fmt(row.end_ms)} ${row.text}` })), 'bg-sky-200 text-sky-950 dark:bg-sky-400/70 dark:text-sky-950')}
      {band(t('Diarizer turns'), (evidence?.diarizer_turns ?? []).map(row => ({ ...row, label: `${row.speaker_key}${row.overlap ? ` · ${t('Overlap')}` : ''}`, title: `${fmt(row.start_ms)}–${fmt(row.end_ms)} ${row.speaker_key}${row.overlap ? ` ${t('Overlap')}` : ''}` })), 'bg-fuchsia-200 text-fuchsia-950 dark:bg-fuchsia-400/70 dark:text-fuchsia-950')}
      {band(t('VAD evidence'), (evidence?.vad_events ?? []).map(row => ({ ...row, label: row.confidence == null ? t('Speech') : row.confidence.toFixed(2), title: `${fmt(row.start_ms)}–${fmt(row.end_ms)} ${t('Confidence')} ${row.confidence ?? t('Not available')}` })), 'bg-emerald-200 text-emerald-950 dark:bg-emerald-400/70 dark:text-emerald-950')}
    </div>;
  }
  const rows = [
    [t('Ground Truth'), t('Visible — manually created canonical events'), 'ground-truth'],
    [t('Speaker helper'), t('Visible — meeting-local descriptions only'), 'helper'],
    [t('System suggestion'), t('Hidden in Blind mode'), 'hidden'],
    [t('ASR transcript'), t('Hidden in Blind mode'), 'hidden'],
    [t('Diarizer turns'), t('Hidden in Blind mode'), 'hidden'],
    [t('VAD evidence'), t('Hidden in Blind mode'), 'hidden'],
  ];
  return <div className="mt-3 divide-y divide-border rounded border border-border text-xs">{rows.map(([name, detail, state]) => <div key={name} className={`grid grid-cols-[9rem_1fr] gap-3 px-3 py-2 ${state === 'ground-truth' ? 'bg-cyan-100/60 dark:bg-cyan-900/20' : 'bg-background/50'}`}><span className="font-semibold text-foreground">{name}</span><span className={state === 'hidden' ? 'text-muted-foreground/70' : 'text-muted-foreground'}>{detail}</span></div>)}</div>;
}

function Inspector({ selected, speakers, update, remove }: { selected: AnnotationEvent | null; speakers: Speaker[]; update: (value: Partial<AnnotationEvent>) => void; remove: () => void }) {
  const { t } = useUiTranslation();
  if (!selected) return <aside className="rounded-xl border border-border bg-card p-4 text-sm text-muted-foreground">{t('Select an event or drag a new region. Every region is a single canonical source-timeline event, never a copy of a window label.')}</aside>;
  const input = (key: 'start_ms' | 'end_ms') => (event: ChangeEvent<HTMLInputElement>) => update({ [key]: Number(event.target.value) });
  const toggle = (key: 'overlap' | 'speaker_handoff' | 'embedded' | 'annotation_uncertain') => () => update({ [key]: !selected[key] });
  const pending = selected.annotation_status === 'pending' || selected.annotation_status === 'review_pending';
  const flags = [
    ['overlap', 'Overlap', 'This event occurs at the same time as another speaker’s real speech.'],
    ['speaker_handoff', 'Speaker handoff', 'This event follows another speaker closely and forms a clear turn transition.'],
    ['embedded', 'Embedded', 'This short event occurs inside a longer utterance, such as a listener saying “mm”.'],
    ['annotation_uncertain', 'Annotation uncertain', 'Enable when the event kind, speaker, or boundary cannot be determined reliably.'],
  ] as const;
  return <aside className="rounded-xl border border-primary/40 bg-card p-3"><div className="mb-3 flex items-start justify-between gap-2"><div><p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Annotation inspector')}</p><p className="font-mono text-xs text-primary">{selected.event_id}</p>{pending && <p className="mt-1 inline-flex rounded bg-warning/15 px-2 py-1 font-semibold text-warning">{t('Needs confirmation')}</p>}</div><button onClick={remove} className="rounded border border-destructive/60 px-2 py-1 text-xs text-destructive hover:bg-destructive/10">{t('Delete')}</button></div><div className="grid grid-cols-2 gap-2"><label className="text-xs text-muted-foreground">{t('Start time (ms)')}<input type="number" value={selected.start_ms} onChange={input('start_ms')} className="mt-1 w-full rounded border border-input bg-background p-2 text-foreground" /></label><label className="text-xs text-muted-foreground">{t('End time (ms)')}<input type="number" value={selected.end_ms} onChange={input('end_ms')} className="mt-1 w-full rounded border border-input bg-background p-2 text-foreground" /></label></div><p className="mt-1 font-mono text-xs text-muted-foreground">{t('Duration')} {selected.end_ms - selected.start_ms} ms · {durationBucket(selected.end_ms - selected.start_ms)}</p><label className="mt-3 block text-xs text-muted-foreground">{t('Kind')}<select value={selected.kind} onChange={event => update({ kind: event.target.value as Kind })} className="mt-1 w-full rounded border border-input bg-background p-2 text-sm text-foreground">{kindKeys.map(kind => <option key={kind.value} value={kind.value}>{t(kind.label)}</option>)}</select></label><p className="mt-1 text-xs text-muted-foreground">{t(kindKeys.find(kind => kind.value === selected.kind)?.help ?? '')}</p><label className="mt-3 block text-xs text-muted-foreground">{t('Meeting-local speaker')}<select value={selected.speaker ?? ''} onChange={event => update({ speaker: event.target.value || null })} className="mt-1 w-full rounded border border-input bg-background p-2 text-sm text-foreground"><option value="">{t('No speaker')}</option>{speakers.map(speaker => <option key={speaker.key} value={speaker.key}>{speaker.key}</option>)}</select></label><label className="mt-3 block text-xs text-muted-foreground">{t('Should this appear on the final timeline?')}<select value={selected.expected_materialized === null ? 'auto' : selected.expected_materialized ? 'yes' : 'no'} onChange={event => update({ expected_materialized: event.target.value === 'auto' ? null : event.target.value === 'yes' })} className="mt-1 w-full rounded border border-input bg-background p-2 text-sm text-foreground"><option value="auto">{t('Decide automatically')}</option><option value="yes">{t('Should appear')}</option><option value="no">{t('Should not appear')}</option></select></label><p className="mt-1 text-xs text-muted-foreground">{t('Usually leave this as “Decide automatically”. The system uses the event kind to decide whether it belongs on the final meeting timeline.')}</p><fieldset className="mt-3 grid grid-cols-2 gap-2"><legend className="mb-1 text-xs text-muted-foreground">{t('Context flags')}</legend>{flags.map(([key, label, help]) => <label key={key} title={t(help)} className="flex items-center gap-2 text-xs"><input type="checkbox" checked={selected[key]} onChange={toggle(key)} />{t(label)}</label>)}</fieldset>{pending && <button onClick={() => update({ annotation_status: selected.annotation_status === 'review_pending' ? 'reviewed' : 'blind_confirmed' })} className="mt-3 w-full rounded bg-success px-3 py-2 text-sm font-semibold text-success-foreground">{t('Confirm annotation')}</button>}<label className="mt-3 block text-xs text-muted-foreground">{t('Notes')}<textarea value={selected.notes} onChange={event => update({ notes: event.target.value })} rows={3} className="mt-1 w-full rounded border border-input bg-background p-2 text-sm text-foreground" /></label></aside>;
}

function QaView({ qa }: { qa: QaReport }) {
  const { t } = useUiTranslation();
  return <div className="mt-3 rounded border border-border bg-background p-3 text-sm"><p className={qa.errors.length ? 'font-semibold text-destructive' : 'font-semibold text-success'}>{qa.errors.length ? t('Blocking QA issues found', { count: qa.errors.length }) : t('Local annotation QA passed')}</p>{qa.errors.map(error => <p key={error} className="mt-1 text-destructive">• {error}</p>)}{qa.possibleDuplicates.map(pair => <p key={`${pair.first_event_id}-${pair.second_event_id}`} className="mt-1 text-warning">{t('Possible duplicate')}: {pair.first_event_id} / {pair.second_event_id} (IoU {pair.overlap_iou.toFixed(2)}) — {t('review manually')}.</p>)}</div>;
}
function CheckView({ check }: { check: CheckReport }) {
  const { t } = useUiTranslation();
  return <div className="mt-3 rounded border border-border bg-background p-3 text-sm"><p className="font-semibold text-primary">{check.representative_data_gate}</p><p className="mt-1 text-muted-foreground">{t('Scorable')}: {String(check.coverage.scorable_samples ?? '—')} · {t('True short')}: {String(check.coverage.true_short_events ?? '—')} · {t('Possible duplicates')}: {check.possible_duplicates.length}</p></div>;
}
function ShortcutHelp() {
  const { t } = useUiTranslation();
  const groups = [
    {
      title: t('Playback and navigation'),
      items: [
        ['Space', t('Play / pause')],
        ['← / →', t('Move backward / forward 100 ms')],
        ['Shift + ← / →', t('Move backward / forward 500 ms')],
        ['J / L', t('Move backward / forward 1 second')],
      ],
    },
    {
      title: t('Classification and flags'),
      items: [
        ['B / S / N / V / C', t('Choose annotation kind')],
        ['1–9', t('Assign speaker')],
        ['O / H / E / U', t('Toggle context flags')],
      ],
    },
    {
      title: t('Workflow and editing'),
      items: [
        ['Enter', t('Complete window')],
        ['Delete', t('Delete event')],
        ['Ctrl/Cmd + Z', t('Undo')],
        ['Ctrl/Cmd + Shift + Z', t('Redo')],
      ],
    },
  ] as const;
  return <details className="group mt-3 rounded-xl border border-border bg-card shadow-sm"><summary className="cursor-pointer select-none px-4 py-3 text-sm font-semibold text-foreground marker:text-muted-foreground">{t('Keyboard shortcuts')}</summary><div className="border-t border-border p-4"><div className="grid gap-3 md:grid-cols-2 min-[900px]:grid-cols-3">{groups.map(group => <section key={group.title} aria-label={group.title} className="rounded-lg border border-border bg-muted/35 p-3"><h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">{group.title}</h3><ul className="space-y-2">{group.items.map(([keys, label]) => <li key={keys} className="grid grid-cols-[minmax(7.5rem,auto)_1fr] items-center gap-3"><kbd className="inline-flex min-h-7 w-fit items-center rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] font-semibold leading-none text-foreground shadow-[0_1px_0_hsl(var(--border))]">{keys}</kbd><span className="text-xs leading-5 text-foreground/80">{label}</span></li>)}</ul></section>)}</div><p className="mt-3 rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">{t('Shortcuts are suspended while entering text or notes.')}</p></div></details>;
}
