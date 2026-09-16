'use client';

import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { WaveformTimeline } from '@/components/short-turn-annotation/WaveformTimeline';
import { isTauri } from '@/lib/isTauri';
import { canShowReconstructionEvidence, UtteranceAnnotationMode } from '@/lib/utteranceAnnotationVisibility';

type Speaker = { key: string; description: string };
type AnnotationStatus = 'pending' | 'confirmed_blind' | 'reviewed';
type SpeakerInterval = { id: string; start_ms: number; end_ms: number; gt_speaker_key: string; overlap: boolean; uncertain: boolean; status: AnnotationStatus };
type Bucket = 'clean_single_speaker' | 'same_speaker_continuity' | 'same_speaker_boundary' | 'asr_fragmentation' | 'vad_fragmentation' | 'speaker_handoff' | 'backchannel' | 'short_speech' | 'true_overlap' | 'noisy_speech' | 'chinese' | 'english' | 'chinese_english_mixed' | 'timing_unavailable_fallback';
type Utterance = { id: string; start_ms: number; end_ms: number; gt_speaker_key: string; boundary_start_uncertain: boolean; boundary_end_uncertain: boolean; contains_backchannel: boolean; overlap: boolean; buckets: Bucket[]; status: AnnotationStatus };
type Boundary = { id: string; timestamp_ms: number; boundary_kind: 'utterance' | 'speaker_handoff' | 'short_speech'; uncertain: boolean; status: AnnotationStatus };
type GroundTruth = { schema_version: number; annotation_version: string; meeting_id: string; reconstruction_artifact_id: string; reconstruction_artifact_sha256: string; source_audio_sha256: string; dataset_split: 'calibration' | 'evaluation'; blind_complete: boolean; review_complete: boolean; qa_passed: boolean; speakers: Speaker[]; speaker_intervals: SpeakerInterval[]; utterances: Utterance[]; boundaries: Boundary[] };
type SystemUtterance = { id: string; start_ms: number; end_ms: number; text: string; speaker_attribution: { kind: string; speaker_key?: string } };
type ReviewEvidence = { system_label: string; raw_transcripts: { id: string; transcript: string; audio_start_time: number | null; audio_end_time: number | null }[]; baseline: { utterances: SystemUtterance[] }; candidate: { utterances: SystemUtterance[]; alignment_diagnostics: unknown[]; timing_diagnostics: unknown[] } };
type View = { meeting_id: string; source_media: { duration_ms: number | null }; ground_truth: GroundTruth; review_evidence: ReviewEvidence | null };

const field = 'w-full rounded border border-border bg-background px-2 py-1 text-sm';
const button = 'rounded border border-border bg-background px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40';

export default function UtteranceReconstructionAnnotationPage() {
  const [artifactPath, setArtifactPath] = useState('');
  const [groundTruthPath, setGroundTruthPath] = useState('');
  const [mediaPath, setMediaPath] = useState('');
  const [mode, setMode] = useState<UtteranceAnnotationMode>('blind');
  const [datasetSplit, setDatasetSplit] = useState<'calibration' | 'evaluation'>('calibration');
  const [view, setView] = useState<View | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [currentMs, setCurrentMs] = useState(0);
  const [history, setHistory] = useState<GroundTruth[]>([]);
  const [future, setFuture] = useState<GroundTruth[]>([]);
  const [dirty, setDirty] = useState(false);
  const [status, setStatus] = useState('');
  const mediaRef = useRef<HTMLAudioElement>(null);
  const revisionRef = useRef(0);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const gt = view?.ground_truth;
  const sourceUrl = mediaPath ? (isTauri() ? convertFileSrc(mediaPath) : mediaPath) : '';

  const change = useCallback((mutation: (value: GroundTruth) => GroundTruth) => {
    setView(current => {
      if (!current) return current;
      revisionRef.current += 1;
      setHistory(items => [...items.slice(-49), current.ground_truth]); setFuture([]); setDirty(true);
      return { ...current, ground_truth: mutation(current.ground_truth) };
    });
  }, []);

  const load = useCallback(async (nextMode: UtteranceAnnotationMode) => {
    try {
      const result = await invoke<View>('api_load_utterance_ground_truth', { request: { artifactPath, groundTruthPath, sourceMediaPath: mediaPath, mode: nextMode } });
      revisionRef.current = 0; setView(result); setMode(nextMode); setSelectedId(null); setDirty(false); setHistory([]); setFuture([]); setStatus(`Loaded ${nextMode}`);
    } catch (error) { setStatus(String(error)); }
  }, [artifactPath, groundTruthPath, mediaPath]);

  const initialize = useCallback(async () => {
    try {
      await invoke('api_initialize_utterance_ground_truth', { request: { artifactPath, groundTruthPath, datasetSplit } });
      await load('blind');
    } catch (error) { setStatus(String(error)); }
  }, [artifactPath, groundTruthPath, datasetSplit, load]);

  const save = useCallback(async () => {
    if (!gt) return;
    const snapshot = gt; const revision = revisionRef.current;
    const operation = saveQueueRef.current.then(() => invoke<void>('api_save_utterance_ground_truth', { request: { artifactPath, groundTruthPath, groundTruth: snapshot } }));
    saveQueueRef.current = operation.catch(() => undefined);
    try { await operation; if (revision === revisionRef.current) { setDirty(false); setStatus('Saved'); } }
    catch (error) { if (revision === revisionRef.current) setStatus(String(error)); }
  }, [artifactPath, groundTruthPath, gt]);
  useEffect(() => { if (!dirty) return; const timer = window.setTimeout(() => void save(), 500); return () => clearTimeout(timer); }, [dirty, save]);

  const addSpeaker = () => change(value => {
    let index = 1; const keys = new Set(value.speakers.map(item => item.key));
    while (keys.has(`gt_speaker_${String(index).padStart(2, '0')}`)) index += 1;
    return { ...value, speakers: [...value.speakers, { key: `gt_speaker_${String(index).padStart(2, '0')}`, description: '' }] };
  });
  const createUtterance = (start_ms: number, end_ms: number) => change(value => {
    const speaker = value.speakers[0]?.key; if (!speaker) return value;
    const id = `${value.meeting_id}-gt-utterance-${String(value.utterances.length + 1).padStart(4, '0')}`;
    return { ...value, utterances: [...value.utterances, { id, start_ms, end_ms, gt_speaker_key: speaker, boundary_start_uncertain: false, boundary_end_uncertain: false, contains_backchannel: false, overlap: false, buckets: [], status: 'pending' }], speaker_intervals: [...value.speaker_intervals, { id: `${id}-speaker`, start_ms, end_ms, gt_speaker_key: speaker, overlap: false, uncertain: false, status: 'pending' }] };
  });
  const selected = gt?.utterances.find(item => item.id === selectedId);
  const waveformEvents = useMemo(() => gt?.utterances.map(item => ({ event_id: item.id, start_ms: item.start_ms, end_ms: item.end_ms, kind: 'ordinary_speech_control' })) ?? [], [gt?.utterances]);
  const updateSelected = (patch: Partial<Utterance>) => change(value => ({ ...value, utterances: value.utterances.map(item => item.id === selectedId ? { ...item, ...patch } : item), speaker_intervals: value.speaker_intervals.map(item => item.id === `${selectedId}-speaker` ? { ...item, gt_speaker_key: patch.gt_speaker_key ?? item.gt_speaker_key, overlap: patch.overlap ?? item.overlap, uncertain: patch.boundary_start_uncertain ?? item.uncertain, status: patch.status ?? item.status } : item) }));
  const addBoundary = (kind: Boundary['boundary_kind']) => change(value => ({ ...value, boundaries: [...value.boundaries, { id: `${value.meeting_id}-boundary-${value.boundaries.length + 1}`, timestamp_ms: currentMs, boundary_kind: kind, uncertain: false, status: 'pending' }] }));
  const markAllForPass = () => change(value => {
    const status: AnnotationStatus = mode === 'review' ? 'reviewed' : 'confirmed_blind';
    return {
      ...value,
      utterances: value.utterances.map(item => ({ ...item, status })),
      speaker_intervals: value.speaker_intervals.map(item => ({ ...item, status })),
      boundaries: value.boundaries.map(item => ({ ...item, status })),
    };
  });
  const undo = () => setHistory(items => { const previous = items.at(-1); if (!previous || !view) return items; revisionRef.current += 1; setFuture(next => [view.ground_truth, ...next]); setView({ ...view, ground_truth: previous }); setDirty(true); return items.slice(0, -1); });
  const redo = () => setFuture(items => { const next = items[0]; if (!next || !view) return items; revisionRef.current += 1; setHistory(previous => [...previous, view.ground_truth]); setView({ ...view, ground_truth: next }); setDirty(true); return items.slice(1); });
  const evidenceVisible = !!gt && canShowReconstructionEvidence(mode, gt.blind_complete);

  return <main className="min-h-screen space-y-4 bg-background p-5 text-foreground">
    <header><h1 className="text-xl font-semibold">Utterance Reconstruction Ground Truth</h1><p className="text-sm text-muted-foreground">Blind → Review → QA → Frozen Baseline vs Candidate benchmark. ASR text is read-only.</p></header>
    <section className="grid gap-2 rounded-xl border border-border bg-card p-4 md:grid-cols-3">
      <input className={field} value={artifactPath} onChange={event => setArtifactPath(event.target.value)} placeholder="Reconstruction artifact path" />
      <input className={field} value={groundTruthPath} onChange={event => setGroundTruthPath(event.target.value)} placeholder="Ground Truth JSON path" />
      <input className={field} value={mediaPath} onChange={event => setMediaPath(event.target.value)} placeholder="Local source media path" />
      <select className={field} value={datasetSplit} onChange={event => setDatasetSplit(event.target.value as 'calibration' | 'evaluation')}><option value="calibration">Calibration set</option><option value="evaluation">Evaluation set</option></select>
      <div className="flex flex-wrap gap-2 md:col-span-3"><button className={button} onClick={initialize}>Initialize</button><button className={button} onClick={() => load('blind')}>Open Blind</button><button className={button} disabled={!gt?.blind_complete} onClick={() => load('review')}>Open Review</button><button className={button} onClick={save} disabled={!dirty}>Save</button><button className={button} onClick={undo} disabled={!history.length}>Undo</button><button className={button} onClick={redo} disabled={!future.length}>Redo</button></div>
      {status && <p className="text-xs text-muted-foreground md:col-span-3">{status}</p>}
    </section>
    {view && <>
      <div className="flex items-center gap-2 rounded-xl border border-border bg-card p-3"><button className={button} onClick={markAllForPass}>{mode === 'review' ? 'Mark all reviewed' : 'Confirm all Blind annotations'}</button><span className="text-xs text-muted-foreground">Pending annotations block pass completion and benchmark export.</span></div>
      <audio ref={mediaRef} src={sourceUrl} controls className="w-full" onLoadedMetadata={() => setCurrentMs(0)} onTimeUpdate={event => setCurrentMs(Math.round(event.currentTarget.currentTime * 1000))} />
      <WaveformTimeline media={mediaRef.current} sourceUrl={sourceUrl} events={waveformEvents} selectedId={selectedId} currentMs={currentMs} onSeek={ms => { setCurrentMs(ms); if (mediaRef.current) mediaRef.current.currentTime = ms / 1000; }} onCreate={createUtterance} onSelect={setSelectedId} onBoundsChange={(id, start_ms, end_ms) => change(value => ({ ...value, utterances: value.utterances.map(item => item.id === id ? { ...item, start_ms, end_ms } : item), speaker_intervals: value.speaker_intervals.map(item => item.id === `${id}-speaker` ? { ...item, start_ms, end_ms } : item) }))} />
      <section className="grid gap-4 lg:grid-cols-3">
        <div className="space-y-2 rounded-xl border border-border bg-card p-4"><h2 className="font-semibold">GT speakers</h2><button className={button} onClick={addSpeaker}>Add meeting-local speaker</button>{gt?.speakers.map(speaker => <div key={speaker.key}><code>{speaker.key}</code><input className={field} value={speaker.description} onChange={event => change(value => ({ ...value, speakers: value.speakers.map(item => item.key === speaker.key ? { ...item, description: event.target.value } : item) }))} placeholder="Local helper description" /></div>)}</div>
        <div className="space-y-2 rounded-xl border border-border bg-card p-4"><h2 className="font-semibold">Selected utterance</h2>{selected ? <><select className={field} value={selected.gt_speaker_key} onChange={event => updateSelected({ gt_speaker_key: event.target.value })}>{gt?.speakers.map(speaker => <option key={speaker.key}>{speaker.key}</option>)}</select><select className={field} value={selected.buckets[0] ?? ''} onChange={event => updateSelected({ buckets: event.target.value ? [event.target.value as Bucket] : [] })}><option value="">Scenario bucket</option>{(['clean_single_speaker', 'same_speaker_continuity', 'same_speaker_boundary', 'asr_fragmentation', 'vad_fragmentation', 'speaker_handoff', 'backchannel', 'short_speech', 'true_overlap', 'noisy_speech', 'chinese', 'english', 'chinese_english_mixed', 'timing_unavailable_fallback'] as Bucket[]).map(value => <option key={value}>{value}</option>)}</select><label><input type="checkbox" checked={selected.overlap} onChange={event => updateSelected({ overlap: event.target.checked })} /> true overlap</label><label className="block"><input type="checkbox" checked={selected.contains_backchannel} onChange={event => updateSelected({ contains_backchannel: event.target.checked })} /> contains backchannel</label><label className="block"><input type="checkbox" checked={selected.boundary_start_uncertain || selected.boundary_end_uncertain} onChange={event => updateSelected({ boundary_start_uncertain: event.target.checked, boundary_end_uncertain: event.target.checked })} /> uncertain boundaries</label><button className={button} onClick={() => updateSelected({ status: mode === 'review' ? 'reviewed' : 'confirmed_blind' })}>{mode === 'review' ? 'Mark reviewed' : 'Confirm Blind'}</button><p className="text-xs">Status: {selected.status}</p><button className={button} onClick={() => change(value => ({ ...value, utterances: value.utterances.filter(item => item.id !== selected.id), speaker_intervals: value.speaker_intervals.filter(item => item.id !== `${selected.id}-speaker`) }))}>Delete</button></> : <p className="text-sm text-muted-foreground">Drag waveform to create an utterance.</p>}</div>
        <div className="space-y-2 rounded-xl border border-border bg-card p-4"><h2 className="font-semibold">Boundaries and workflow</h2><div className="flex flex-wrap gap-2"><button className={button} onClick={() => addBoundary('utterance')}>Utterance boundary</button><button className={button} onClick={() => addBoundary('speaker_handoff')}>Handoff</button><button className={button} onClick={() => addBoundary('short_speech')}>Short speech</button></div><p className="text-xs">At {(currentMs / 1000).toFixed(3)}s · {gt?.boundaries.length ?? 0} boundaries</p><div className="max-h-40 space-y-1 overflow-auto">{gt?.boundaries.map(boundary => <div key={boundary.id} className="flex items-center gap-1"><input className="w-24 rounded border bg-background px-1 text-xs" type="number" value={boundary.timestamp_ms} onChange={event => change(value => ({ ...value, boundaries: value.boundaries.map(item => item.id === boundary.id ? { ...item, timestamp_ms: Number(event.target.value) } : item) }))} /><span className="flex-1 text-xs">{boundary.boundary_kind}</span><label className="text-xs"><input type="checkbox" checked={boundary.uncertain} onChange={event => change(value => ({ ...value, boundaries: value.boundaries.map(item => item.id === boundary.id ? { ...item, uncertain: event.target.checked } : item) }))} /> uncertain</label><button className={button} onClick={() => change(value => ({ ...value, boundaries: value.boundaries.filter(item => item.id !== boundary.id) }))}>×</button></div>)}</div><label className="block"><input type="checkbox" checked={gt?.blind_complete ?? false} onChange={event => change(value => ({ ...value, blind_complete: event.target.checked, review_complete: event.target.checked ? value.review_complete : false, qa_passed: false }))} /> Blind complete</label><label className="block"><input type="checkbox" disabled={mode !== 'review'} checked={gt?.review_complete ?? false} onChange={event => change(value => ({ ...value, review_complete: event.target.checked, qa_passed: false }))} /> Review complete</label><label className="block"><input type="checkbox" disabled={!gt?.blind_complete || !gt?.review_complete} checked={gt?.qa_passed ?? false} onChange={event => change(value => ({ ...value, qa_passed: event.target.checked }))} /> QA passed</label></div>
      </section>
      {mode === 'blind' && <section data-testid="blind-no-system-evidence" className="rounded-xl border border-border bg-card p-4 text-sm">Blind mode: waveform and human Ground Truth only. Frozen Baseline/Candidate, ASR attribution, alignment and boundary predictions are not loaded.</section>}
      {evidenceVisible && view.review_evidence && <section data-testid="review-system-evidence" className="space-y-3 rounded-xl border border-amber-500/50 bg-card p-4"><h2 className="font-semibold text-amber-600">{view.review_evidence.system_label}</h2><p className="text-sm">Frozen Baseline utterances: {view.review_evidence.baseline.utterances.length} · Candidate utterances: {view.review_evidence.candidate.utterances.length} · Timing diagnostics: {view.review_evidence.candidate.timing_diagnostics.length} · Alignment diagnostics: {view.review_evidence.candidate.alignment_diagnostics.length}</p><div className="grid gap-2 md:grid-cols-2"><pre className="max-h-80 overflow-auto rounded bg-muted p-2 text-xs">{JSON.stringify(view.review_evidence.baseline.utterances, null, 2)}</pre><pre className="max-h-80 overflow-auto rounded bg-muted p-2 text-xs">{JSON.stringify(view.review_evidence.candidate.utterances, null, 2)}</pre></div></section>}
    </>}
  </main>;
}
