'use client';

import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import {
  ChangeEvent, KeyboardEvent as ReactKeyboardEvent, PointerEvent, useCallback, useEffect,
  useMemo, useRef, useState,
} from 'react';
import { toast } from 'sonner';
import { AlertTriangle, ChevronLeft, ChevronRight, CircleHelp, Download, Pause, Play, Redo2, Undo2 } from 'lucide-react';
import { isTauri } from '@/lib/isTauri';
import { WaveformTimeline } from '@/components/short-turn-annotation/WaveformTimeline';
import { durationBucket, saveLabel } from '@/lib/annotationIntegrity';

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

const kinds: { value: Kind; label: string }[] = [
  { value: 'short_speech', label: 'Short speech' }, { value: 'backchannel', label: 'Backchannel' },
  { value: 'noise', label: 'Noise' }, { value: 'non_speech_vocalization', label: 'Non-speech vocalization' },
  { value: 'ordinary_speech_control', label: 'Ordinary speech control' },
];

const defaultDraft = (meeting = ''): Draft => ({ schema_version: 1, meeting_id: meeting, events: [] });
const defaultSession = (meeting = ''): Session => ({ schema_version: 1, meeting_id: meeting, source_media_path: '', production_artifact_path: '', speaker_map: [], window_status: {}, next_event_sequence: 1 });
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
const fmt = (value: number) => `${(value / 1000).toFixed(3)}s`;

function mediaUrl(path: string) {
  if (!path) return '';
  return isTauri() ? convertFileSrc(path) : path;
}

function isTextEntry(target: EventTarget | null) {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** Deliberately unlinked development/evaluation surface.  Production users do not see it. */
export default function ShortTurnAnnotationPage() {
  const enabled = process.env.NODE_ENV === 'development' || process.env.NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION === 'true';
  const [datasetDir, setDatasetDir] = useState('evaluation/short_turn_dataset/local');
  const [meetingId, setMeetingId] = useState('');
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

  const currentWindow = windows[windowIndex];
  const selected = draft.events.find(event => event.event_id === selectedId) ?? null;
  const viewportStart = currentWindow?.source_start_ms ?? 0;
  const viewportEnd = currentWindow?.source_end_ms ?? 5000;
  const viewportDuration = Math.max(1, viewportEnd - viewportStart);
  const displayedEvents = useMemo(() => draft.events.filter(event => event.start_ms < viewportEnd && event.end_ms > viewportStart).sort((a, b) => a.start_ms - b.start_ms), [draft.events, viewportStart, viewportEnd]);
  const video = /\.(mp4|webm)$/i.test(session.source_media_path);

  const markDirty = useCallback(() => { setEditRevision(value => { const next = value + 1; editRevisionRef.current = next; return next; }); setQa(null); setQaRevision(null); setCheck(null); }, []);
  const commit = useCallback((mutation: (current: Draft) => Draft) => {
    setDraft(current => { setHistory(history => [...history.slice(-49), current]); setFuture([]); markDirty(); return mutation(current); });
  }, [markDirty]);
  const updateSession = useCallback((mutation: (current: Session) => Session) => {
    setSession(current => mutation(current)); markDirty();
  }, [markDirty]);

  const load = useCallback(async (nextPass = annotationPass) => {
    if (!datasetDir.trim() || !meetingId.trim()) return toast.error('Dataset folder and Meeting ID are required');
    try {
      const result = await invoke<Snapshot>('load_workspace', { request: { datasetDir, meetingId, mode: nextPass } });
      setDraft(result.draft); setSession(result.session); draftRef.current=result.draft; sessionRef.current=result.session; allocatorRef.current=result.session.next_event_sequence || 1; setWindows(result.windows); setReviewEvidence(result.reviewEvidence); setAnnotationPass(nextPass); setPanel('annotation'); setInitialized(result.initialized); const lastId = nextPass === 'blind' ? result.session.last_blind_window_id : result.session.last_review_window_id; const restored = result.windows.findIndex(row => row.window_id === lastId); setWindowIndex(restored >= 0 ? restored : 0); setSelectedId(null); setHistory([]); setFuture([]); setEditRevision(0); editRevisionRef.current=0; setSavedRevision(0); setSavingRevision(null); setSaveFailed(false); setQa(null); setQaRevision(null); setCheck(null);
    } catch (error) { toast.error(String(error)); }
  }, [datasetDir, meetingId, annotationPass]);

  const flushSave = useCallback(async () => {
    if (!isTauri() || !initialized || saveInFlightRef.current || editRevisionRef.current <= savedRevision) return;
    saveInFlightRef.current = true; const revision = editRevisionRef.current; setSavingRevision(revision); setSaveFailed(false);
    try { await invoke('save_workspace', { request: { datasetDir, draft: draftRef.current, session: sessionRef.current } }); setSavedRevision(value => Math.max(value, revision)); }
    catch (error) { setSaveFailed(true); toast.error(`Save failed: ${String(error)}`); }
    finally { saveInFlightRef.current = false; setSavingRevision(null); if (editRevisionRef.current > revision) window.setTimeout(() => void flushSave(), 0); }
  }, [datasetDir, initialized, savedRevision]);
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
  const exportManifest = useCallback(async () => { try { const result = await invoke<CheckReport>('export_manifest_command', { datasetDir, draft, session }); setCheck(result); toast.success('Benchmark manifest exported and checked locally'); } catch (error) { toast.error(String(error)); } }, [datasetDir, draft, session]);
  const goToWindow = useCallback((index: number) => { const next = clamp(index, 0, Math.max(0, windows.length - 1)); setWindowIndex(next); const row=windows[next]; if (row) { seek(row.source_start_ms); updateSession(current => ({ ...current, [annotationPass === 'blind' ? 'last_blind_window_id' : 'last_review_window_id']: row.window_id })); } }, [annotationPass, seek, updateSession, windows]);
  const completeWindow = useCallback(() => { if (!currentWindow) return; const status = annotationPass === 'blind' ? 'reviewed_blind' : 'reviewed_second_pass'; updateSession(current => ({ ...current, window_status: { ...current.window_status, [currentWindow.window_id]: status } })); const next = windows.findIndex((row, index) => index > windowIndex && session.window_status[row.window_id] !== status); if (next >= 0) goToWindow(next); }, [annotationPass, currentWindow, goToWindow, session.window_status, updateSession, windowIndex, windows]);
  const initializeProject = useCallback(async (sourceMediaPath: string, productionArtifactPath: string) => {
    if (!datasetDir.trim() || !meetingId.trim()) return toast.error('Dataset folder and Meeting ID are required');
    try {
      const result = await invoke<Snapshot>('initialize_annotation_project', { request: { datasetDir, meetingId, sourceMediaPath, productionArtifactPath } });
      setDraft(result.draft); setSession(result.session); draftRef.current=result.draft; sessionRef.current=result.session; allocatorRef.current=result.session.next_event_sequence; setWindows(result.windows); setReviewEvidence(null); setInitialized(true); setAnnotationPass('blind'); setPanel('annotation'); setWindowIndex(0); setEditRevision(0); setSavedRevision(0); toast.success('Annotation project initialized');
    } catch (error) { toast.error(String(error)); }
  }, [datasetDir, meetingId]);

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

  if (!enabled) return <main className="p-10"><h1 className="text-xl font-semibold">Development evaluation route is disabled</h1><p className="mt-2 text-muted-foreground">Set <code>NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION=true</code> in a development/evaluation build.</p></main>;

  return <main className="min-h-screen bg-slate-950 p-4 text-slate-100 selection:bg-cyan-300 selection:text-slate-950">
    <header className="mb-3 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 shadow-lg shadow-black/20">
      <div><p className="text-[11px] font-bold tracking-[0.18em] text-cyan-300">HUITRACE / PHASE 2D.2</p><h1 className="text-lg font-semibold">Local Short-Turn Annotation Workspace</h1></div>
      <div className="flex flex-wrap items-center gap-2 text-sm"><span className={`rounded px-2 py-1 font-bold tracking-wide ${annotationPass === 'blind' ? 'bg-amber-400 text-slate-950' : 'bg-violet-300 text-slate-950'}`}>{annotationPass === 'blind' ? 'BLIND ANNOTATION' : 'REVIEW ANNOTATION'}</span><span className="rounded border border-slate-700 px-2 py-1 text-xs">{panel.toUpperCase()}</span><span className="font-mono text-slate-300">{meetingId || 'No meeting'}</span><span className="font-mono text-cyan-200">{fmt(currentMs)}</span><span aria-live="polite" className={saveFailed ? 'text-rose-300' : editRevision === savedRevision && savingRevision === null ? 'text-emerald-300' : 'text-amber-200'}>{saveLabel(editRevision, savedRevision, savingRevision, saveFailed)}</span></div>
    </header>
    <section className="mb-3 grid gap-2 rounded-xl border border-slate-800 bg-slate-900/70 p-3 md:grid-cols-[1.3fr_1fr_1fr_auto]">
      <label className="text-xs text-slate-300">Dataset directory<input value={datasetDir} onChange={event => setDatasetDir(event.target.value)} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1.5 font-mono text-sm" /></label>
      <label className="text-xs text-slate-300">Meeting ID<input value={meetingId} onChange={event => setMeetingId(event.target.value)} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1.5 font-mono text-sm" /></label>
      <div className="flex items-end gap-2"><button onClick={() => load('blind')} className="rounded bg-cyan-300 px-3 py-2 text-sm font-semibold text-slate-950">Open blind pass</button><button onClick={() => load('review')} className="rounded border border-violet-300 px-3 py-2 text-sm text-violet-100">Review</button></div>
      <p className="self-end text-xs text-slate-400">Local files only · no telemetry · no upload</p>
    </section>
    {!initialized ? <Setup initialize={initializeProject} /> : <>
      <section className="grid min-h-[560px] gap-3 xl:grid-cols-[minmax(280px,1.1fr)_minmax(300px,1fr)_minmax(300px,1fr)]">
        <aside className="rounded-xl border border-slate-800 bg-slate-900 p-3">
          <p className="mb-2 text-xs font-semibold uppercase tracking-widest text-slate-400">Source media</p>
          {video ? <video ref={audioRef as React.RefObject<HTMLVideoElement>} src={mediaUrl(session.source_media_path)} controls onLoadedMetadata={event => updateSession(current => ({ ...current, source_media_duration_ms: Math.round(event.currentTarget.duration * 1000) }))} onTimeUpdate={onTimeUpdate} onPlay={() => setIsPlaying(true)} onPause={() => setIsPlaying(false)} className="aspect-video w-full rounded bg-black" /> : <div className="rounded bg-slate-950 p-5"><audio ref={audioRef as React.RefObject<HTMLAudioElement>} src={mediaUrl(session.source_media_path)} controls onLoadedMetadata={event => updateSession(current => ({ ...current, source_media_duration_ms: Math.round(event.currentTarget.duration * 1000) }))} onTimeUpdate={onTimeUpdate} onPlay={() => setIsPlaying(true)} onPause={() => setIsPlaying(false)} className="w-full" /><p className="mt-4 text-sm text-slate-400">Audio-only source. Speaker map descriptions remain local-only.</p></div>}
          <div className="mt-3 flex flex-wrap gap-2"><button onClick={togglePlay} className="inline-flex items-center gap-1 rounded bg-slate-700 px-3 py-2 text-sm">{isPlaying ? <Pause size={16} /> : <Play size={16} />}{isPlaying ? 'Pause' : 'Play'}</button>{[250, 500, 1000].map(padding => <button key={padding} onClick={() => playSelection(padding)} disabled={!selected} className="rounded border border-slate-700 px-2 py-1 text-xs disabled:opacity-40">Event ±{padding}ms</button>)}<button onClick={() => setLoop(value => !value)} disabled={!selected} aria-pressed={loop} className={`rounded border px-2 py-1 text-xs disabled:opacity-40 ${loop ? 'border-cyan-300 text-cyan-200' : 'border-slate-700'}`}>Loop</button></div>
          <div className="mt-5 border-t border-slate-800 pt-3"><p className="mb-2 text-xs font-semibold uppercase tracking-widest text-slate-400">Speaker map — local only</p>{session.speaker_map.map((speaker, index) => <div className="mb-2 grid grid-cols-[8rem_1fr] gap-2" key={speaker.key}><span className="rounded bg-slate-800 p-1 text-center font-mono text-xs">{speaker.key}</span><input value={speaker.description} placeholder="Local description" aria-label={`Speaker ${index + 1} local description`} onChange={event => updateSession(current => ({ ...current, speaker_map: current.speaker_map.map((item, itemIndex) => itemIndex === index ? { ...item, description: event.target.value } : item) }))} className="min-w-0 rounded border border-slate-700 bg-slate-950 px-2 text-sm" /></div>)}<button onClick={() => updateSession(current => ({ ...current, speaker_map: [...current.speaker_map, { key: `gt_speaker_${String(current.speaker_map.length + 1).padStart(2, '0')}`, description: '' }] }))} className="text-sm text-cyan-200 underline">Add speaker</button></div>
        </aside>
        <section className="rounded-xl border border-slate-800 bg-slate-900 p-3"><div className="mb-3 flex items-center justify-between"><div><p className="text-xs font-semibold uppercase tracking-widest text-slate-400">Canonical events</p><p className="text-sm text-slate-300">Window {windows.length ? `${windowIndex + 1} / ${windows.length}` : '—'} · viewport {fmt(viewportStart)}–{fmt(viewportEnd)}</p></div><div className="flex gap-1"><button aria-label="Previous window" onClick={() => goToWindow(windowIndex - 1)} disabled={!windowIndex} className="rounded p-2 hover:bg-slate-800 disabled:opacity-30"><ChevronLeft /></button><button aria-label="Next window" onClick={() => goToWindow(windowIndex + 1)} disabled={windowIndex >= windows.length - 1} className="rounded p-2 hover:bg-slate-800 disabled:opacity-30"><ChevronRight /></button></div></div>
          <div className="space-y-2 overflow-y-auto pr-1 xl:max-h-[490px]">{displayedEvents.length === 0 ? <p className="rounded border border-dashed border-slate-700 p-5 text-sm text-slate-400">Drag a region in the source timeline to create one canonical event.</p> : displayedEvents.map(event => <button key={event.event_id} onClick={() => setSelectedId(event.event_id)} className={`w-full rounded-lg border p-3 text-left transition ${selectedId === event.event_id ? 'border-cyan-300 bg-cyan-300/10' : 'border-slate-800 bg-slate-950 hover:border-slate-600'}`}><div className="flex justify-between gap-2"><span className="font-mono text-xs text-slate-400">{event.event_id}</span><span className="rounded bg-slate-800 px-1.5 text-xs">{durationBucket(event.end_ms - event.start_ms)}</span></div><p className="mt-1 font-medium">{kinds.find(kind => kind.value === event.kind)?.label} <span className="font-normal text-slate-400">· {event.speaker ?? 'no speaker'}</span></p><p className="mt-1 font-mono text-xs text-slate-400">{fmt(event.start_ms)}–{fmt(event.end_ms)} · {event.end_ms - event.start_ms} ms {event.overlap && '· overlap'} {event.speaker_handoff && '· handoff'} {event.embedded && '· embedded'} {event.annotation_uncertain && '· uncertain'}</p></button>)}</div>
          <div className="mt-3 flex flex-wrap gap-2 border-t border-slate-800 pt-3"><button onClick={undo} disabled={!history.length} className="inline-flex items-center gap-1 rounded border border-slate-700 px-2 py-1 text-xs disabled:opacity-30"><Undo2 size={14} />Undo</button><button onClick={redo} disabled={!future.length} className="inline-flex items-center gap-1 rounded border border-slate-700 px-2 py-1 text-xs disabled:opacity-30"><Redo2 size={14} />Redo</button><button onClick={completeWindow} className="rounded bg-emerald-300 px-2 py-1 text-xs font-semibold text-slate-950">Mark window complete</button></div>
        </section>
        <Inspector selected={selected} speakers={session.speaker_map} update={updateSelected} remove={deleteSelected} />
      </section>
      <WaveformTimeline media={audioRef.current} sourceUrl={mediaUrl(session.source_media_path)} events={draft.events} selectedId={selectedId} currentMs={currentMs} onSeek={seek} onCreate={createEvent} onSelect={setSelectedId} onBoundsChange={(id, start_ms, end_ms) => commit(current => ({ ...current, events: current.events.map(event => event.event_id === id ? { ...event, start_ms, end_ms } : event) }))} />
      <section className="mt-3 rounded-xl border border-slate-800 bg-slate-900 p-3"><div className="mb-2 flex flex-wrap items-center justify-between gap-2"><div><p className="text-xs font-semibold uppercase tracking-widest text-slate-400">Multi-tier source timeline</p><p className="text-xs text-slate-400">Ground truth is shown separately from every review-only system evidence tier.</p></div>{annotationPass === 'review' && <span className="rounded border border-violet-300/70 bg-violet-300/10 px-2 py-1 text-xs font-bold tracking-wide text-violet-100">SYSTEM SUGGESTION — NOT GROUND TRUTH</span>}</div>
        <div ref={timelineRef} role="slider" aria-label="Source timeline" aria-valuemin={viewportStart} aria-valuemax={viewportEnd} aria-valuenow={currentMs} tabIndex={0} onPointerDown={onTimelineDown} onPointerUp={onTimelineUp} className="relative h-28 cursor-crosshair overflow-hidden rounded-lg border border-slate-700 bg-slate-950 touch-none">
          <div className="absolute inset-x-0 bottom-0 flex h-14 items-end gap-[3px] px-1 opacity-35" aria-hidden="true">{Array.from({ length: 110 }, (_, index) => <span key={index} style={{ height: `${20 + ((index * 37) % 70)}%` }} className="flex-1 rounded-t bg-cyan-300" />)}</div>
          {displayedEvents.map(event => <Region key={event.event_id} event={event} selected={event.event_id === selectedId} start={viewportStart} duration={viewportDuration} onSelect={() => setSelectedId(event.event_id)} onEdge={dragEdge} />)}
          {annotationPass === 'review' && currentWindow?.candidate_suggestions.map((suggestion, index) => <button key={index} type="button" title="Accept suggestion as a pending editable annotation" onClick={() => createEvent(suggestion.start_ms, suggestion.end_ms)} className="absolute top-5 h-5 border border-dashed border-violet-300/80 bg-violet-300/10" style={{ left: `${clamp((suggestion.start_ms - viewportStart) / viewportDuration * 100, 0, 100)}%`, width: `${clamp((suggestion.end_ms - suggestion.start_ms) / viewportDuration * 100, 0.3, 100)}%` }}><span className="absolute -top-4 whitespace-nowrap text-[9px] font-bold text-violet-200">SYSTEM · accept pending</span></button>)}
          <div className="absolute inset-y-0 w-px bg-amber-300" style={{ left: `${clamp((currentMs - viewportStart) / viewportDuration * 100, 0, 100)}%` }} />
        </div>
        <TierRows mode={annotationPass} evidence={reviewEvidence} start={viewportStart} end={viewportEnd} />
      </section>
      <section className="mt-3 grid gap-3 rounded-xl border border-slate-800 bg-slate-900 p-3 lg:grid-cols-[1fr_auto]"><div><p className="text-xs font-semibold uppercase tracking-widest text-slate-400">QA / benchmark export</p><p className="mt-1 text-sm text-slate-300">Blind QA is structural only. Dataset coverage and representative gates remain hidden until Review.</p>{qa && <QaView qa={qa} />}{annotationPass === 'review' && check && <CheckView check={check} />}</div><div className="flex flex-wrap content-start gap-2"><button onClick={runQa} className="inline-flex items-center gap-1 rounded border border-amber-300 px-3 py-2 text-sm text-amber-100"><CircleHelp size={16} />Run QA</button>{annotationPass === 'review' && <button onClick={exportManifest} disabled={!qa || qa.errors.length > 0 || qaRevision !== editRevision || savedRevision !== editRevision} className="inline-flex items-center gap-1 rounded bg-cyan-300 px-3 py-2 text-sm font-semibold text-slate-950 disabled:opacity-40"><Download size={16} />Export Benchmark Manifest</button>}</div></section>
    </>}
    <ShortcutHelp />
  </main>;
}

function Setup({ initialize }: { initialize: (source: string, artifact: string) => void }) {
  const [source, setSource] = useState(''); const [artifact, setArtifact] = useState('');
  return <section className="mx-auto max-w-2xl rounded-xl border border-cyan-300/50 bg-slate-900 p-6"><h2 className="text-lg font-semibold">Initialize annotation project</h2><p className="mt-1 text-sm text-slate-400">Initialization validates both window exports and the immutable Production Artifact, then copies media into HuiTrace&apos;s controlled audio directory.</p><label className="mt-4 block text-sm">Source media (WAV, MP3, M4A, MP4, or WebM)<input value={source} onChange={event => setSource(event.target.value)} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 font-mono text-sm" /></label><label className="mt-3 block text-sm">Production artifact JSON<input value={artifact} onChange={event => setArtifact(event.target.value)} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 font-mono text-sm" /></label><button disabled={!source.trim() || !artifact.trim()} onClick={() => initialize(source, artifact)} className="mt-4 rounded bg-cyan-300 px-4 py-2 font-semibold text-slate-950 disabled:opacity-40">Initialize Project</button><p className="mt-4 rounded bg-amber-300/10 p-3 text-sm text-amber-100"><AlertTriangle className="mr-1 inline" size={16} />Review stays locked until every expected Blind window is complete.</p></section>;
}

function Region({ event, selected, start, duration, onSelect, onEdge }: { event: AnnotationEvent; selected: boolean; start: number; duration: number; onSelect: () => void; onEdge: (event: PointerEvent<HTMLButtonElement>, edge: 'start' | 'end', item: AnnotationEvent) => void }) {
  const left = clamp((event.start_ms - start) / duration * 100, 0, 100); const width = clamp((event.end_ms - event.start_ms) / duration * 100, 0.5, 100);
  return <div data-region onPointerDown={event => { event.stopPropagation(); onSelect(); }} className={`absolute top-12 h-8 rounded ${selected ? 'bg-cyan-300/70 ring-2 ring-cyan-100' : 'bg-cyan-400/45'}`} style={{ left: `${left}%`, width: `${width}%` }}><button aria-label={`Adjust start of ${event.event_id}`} onPointerDown={pointer => onEdge(pointer, 'start', event)} className="absolute -left-1 top-0 h-full w-2 cursor-ew-resize rounded bg-cyan-100" /><span className="pointer-events-none px-1 text-[10px] font-bold text-slate-950">{event.kind}</span><button aria-label={`Adjust end of ${event.event_id}`} onPointerDown={pointer => onEdge(pointer, 'end', event)} className="absolute -right-1 top-0 h-full w-2 cursor-ew-resize rounded bg-cyan-100" /></div>;
}

function TierRows({ mode, evidence, start, end }: { mode: AnnotationPass; evidence: ReviewEvidence | null; start: number; end: number }) {
  if (mode === 'review') {
    const duration = Math.max(1, end - start);
    const band = (name: string, rows: { start_ms: number; end_ms: number; label: string; title: string }[], color: string) => <div className="grid grid-cols-[9rem_1fr] gap-3 px-3 py-2"><span className="font-semibold text-slate-200">{name}</span><div className="relative h-7 overflow-hidden rounded bg-slate-900">{rows.filter(row => row.start_ms < end && row.end_ms > start).map((row, index) => <span key={`${row.start_ms}-${row.end_ms}-${index}`} title={row.title} className={`absolute inset-y-1 overflow-hidden whitespace-nowrap rounded px-1 text-[9px] text-slate-950 ${color}`} style={{ left: `${clamp((row.start_ms-start)/duration*100,0,100)}%`, width: `${clamp((row.end_ms-row.start_ms)/duration*100,.4,100)}%` }}>{row.label}</span>)}</div></div>;
    return <div className="mt-3 divide-y divide-slate-800 rounded border border-slate-800 text-xs">
      {band('ASR transcript', (evidence?.transcripts ?? []).map(row => ({ ...row, label: row.text, title: `${fmt(row.start_ms)}–${fmt(row.end_ms)} ${row.text}` })), 'bg-sky-300')}
      {band('Diarizer turns', (evidence?.diarizer_turns ?? []).map(row => ({ ...row, label: `${row.speaker_key}${row.overlap ? ' · overlap' : ''}`, title: `${fmt(row.start_ms)}–${fmt(row.end_ms)} ${row.speaker_key}${row.overlap ? ' overlap' : ''}` })), 'bg-fuchsia-300')}
      {band('VAD evidence', (evidence?.vad_events ?? []).map(row => ({ ...row, label: row.confidence == null ? 'speech' : row.confidence.toFixed(2), title: `${fmt(row.start_ms)}–${fmt(row.end_ms)} confidence ${row.confidence ?? 'n/a'}` })), 'bg-emerald-300')}
    </div>;
  }
  const rows = [
    ['Ground Truth', 'Visible — manually created canonical events'],
    ['Speaker helper', 'Visible — meeting-local descriptions only'],
    ['System Prediction', 'Hidden in blind mode'],
    ['ASR transcript', 'Hidden in blind mode'],
    ['Diarizer turns', 'Hidden in blind mode'],
    ['VAD evidence', 'Hidden in blind mode'],
  ];
  return <div className="mt-3 divide-y divide-slate-800 rounded border border-slate-800 text-xs">{rows.map(([name, detail]) => <div key={name} className={`grid grid-cols-[9rem_1fr] gap-3 px-3 py-2 ${name === 'Ground Truth' ? 'bg-cyan-300/5' : 'bg-slate-950/50'}`}><span className="font-semibold text-slate-200">{name}</span><span className={detail.includes('Hidden') ? 'text-slate-500' : name === 'System Prediction' ? 'font-semibold text-violet-200' : 'text-slate-400'}>{detail}</span></div>)}</div>;
}

function Inspector({ selected, speakers, update, remove }: { selected: AnnotationEvent | null; speakers: Speaker[]; update: (value: Partial<AnnotationEvent>) => void; remove: () => void }) {
  if (!selected) return <aside className="rounded-xl border border-slate-800 bg-slate-900 p-4 text-sm text-slate-400">Select an event or drag a new region. Every region is a single canonical source-timeline event, never a copy of a window label.</aside>;
  const input = (key: 'start_ms' | 'end_ms') => (event: ChangeEvent<HTMLInputElement>) => update({ [key]: Number(event.target.value) });
  const toggle = (key: 'overlap' | 'speaker_handoff' | 'embedded' | 'annotation_uncertain') => () => update({ [key]: !selected[key] });
  const pending = selected.annotation_status === 'pending' || selected.annotation_status === 'review_pending';
  return <aside className="rounded-xl border border-cyan-300/50 bg-slate-900 p-3"><div className="mb-3 flex items-start justify-between gap-2"><div><p className="text-xs font-semibold uppercase tracking-widest text-slate-400">Annotation inspector</p><p className="font-mono text-xs text-cyan-200">{selected.event_id}</p>{pending && <p className="mt-1 font-semibold text-amber-200">Needs confirmation</p>}</div><button onClick={remove} className="rounded border border-rose-400/70 px-2 py-1 text-xs text-rose-200">Delete</button></div><div className="grid grid-cols-2 gap-2"><label className="text-xs text-slate-400">Start ms<input type="number" value={selected.start_ms} onChange={input('start_ms')} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 text-slate-100" /></label><label className="text-xs text-slate-400">End ms<input type="number" value={selected.end_ms} onChange={input('end_ms')} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 text-slate-100" /></label></div><p className="mt-1 font-mono text-xs text-slate-400">Duration {selected.end_ms - selected.start_ms} ms · {durationBucket(selected.end_ms - selected.start_ms)}</p><label className="mt-3 block text-xs text-slate-400">Kind<select value={selected.kind} onChange={event => update({ kind: event.target.value as Kind })} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-100">{kinds.map(kind => <option key={kind.value} value={kind.value}>{kind.label}</option>)}</select></label><label className="mt-3 block text-xs text-slate-400">Meeting-local speaker<select value={selected.speaker ?? ''} onChange={event => update({ speaker: event.target.value || null })} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-100"><option value="">No speaker</option>{speakers.map(speaker => <option key={speaker.key} value={speaker.key}>{speaker.key}</option>)}</select></label><label className="mt-3 block text-xs text-slate-400">Expected materialized<select value={selected.expected_materialized === null ? 'auto' : selected.expected_materialized ? 'yes' : 'no'} onChange={event => update({ expected_materialized: event.target.value === 'auto' ? null : event.target.value === 'yes' })} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-100"><option value="auto">Auto</option><option value="yes">Yes</option><option value="no">No</option></select></label><fieldset className="mt-3 grid grid-cols-2 gap-2"><legend className="mb-1 text-xs text-slate-400">Tags</legend>{([['overlap', 'Overlap'], ['speaker_handoff', 'Speaker handoff'], ['embedded', 'Embedded'], ['annotation_uncertain', 'Annotation uncertain']] as const).map(([key, label]) => <label key={key} className="flex items-center gap-2 text-xs"><input type="checkbox" checked={selected[key]} onChange={toggle(key)} />{label}</label>)}</fieldset>{pending && <button onClick={() => update({ annotation_status: selected.annotation_status === 'review_pending' ? 'reviewed' : 'blind_confirmed' })} className="mt-3 w-full rounded bg-emerald-300 px-3 py-2 text-sm font-semibold text-slate-950">Confirm annotation</button>}<label className="mt-3 block text-xs text-slate-400">Notes<textarea value={selected.notes} onChange={event => update({ notes: event.target.value })} rows={3} className="mt-1 w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-100" /></label></aside>;
}

function QaView({ qa }: { qa: QaReport }) { return <div className="mt-3 rounded border border-slate-700 bg-slate-950 p-3 text-sm"><p className={qa.errors.length ? 'font-semibold text-rose-200' : 'font-semibold text-emerald-200'}>{qa.errors.length ? `${qa.errors.length} blocking QA issue(s)` : 'Local annotation QA passed'}</p>{qa.errors.map(error => <p key={error} className="mt-1 text-rose-200">• {error}</p>)}{qa.possibleDuplicates.map(pair => <p key={`${pair.first_event_id}-${pair.second_event_id}`} className="mt-1 text-amber-100">Possible duplicate: {pair.first_event_id} / {pair.second_event_id} (IoU {pair.overlap_iou.toFixed(2)}) — review manually.</p>)}</div>; }
function CheckView({ check }: { check: CheckReport }) { return <div className="mt-3 rounded border border-slate-700 bg-slate-950 p-3 text-sm"><p className="font-semibold text-cyan-100">{check.representative_data_gate}</p><p className="mt-1 text-slate-300">Scorable: {String(check.coverage.scorable_samples ?? '—')} · True short: {String(check.coverage.true_short_events ?? '—')} · Possible duplicates: {check.possible_duplicates.length}</p></div>; }
function ShortcutHelp() { return <details className="mt-3 text-xs text-slate-400"><summary className="cursor-pointer">Keyboard shortcuts</summary><p className="mt-1">Space play/pause · ←/→ ±100 ms · Shift+←/→ ±500 ms · J/L ±1 s · B/S/N/V/C kind · 1–9 speaker · O/H/E/U flags · Enter complete window · Delete event · Ctrl/Cmd+Z undo · Ctrl/Cmd+Shift+Z redo. Shortcuts are suspended while entering notes.</p></details>; }
