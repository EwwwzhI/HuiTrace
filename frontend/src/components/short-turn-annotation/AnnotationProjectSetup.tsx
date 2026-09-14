"use client";

import { useId, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { AlertTriangle, Check, FileAudio, FileJson, FolderOpen, Loader2 } from 'lucide-react';
import { toast } from 'sonner';

import { useUiTranslation } from '@/i18n/client';

export type ProductionArtifactInspection = {
  meetingId: string;
  artifactId: string;
  schemaVersion: number;
  transcriptionRunId: string;
  durationMs: number;
  asrBackend: string;
  asrModel: string;
  diarizationBackend: string;
  diarizationModel: string;
  transcriptCount: number;
  diarizerTurnCount: number;
  vadEventCount: number;
};

export type AnnotationPreparationResult = {
  meetingId: string;
  blindWindowCount: number;
  reviewWindowCount: number;
  candidateCount: number;
  controlledProductionArtifact: string;
  blindManifest: string;
  reviewManifest: string;
};

type Props = {
  datasetRoot: string;
  setDatasetRoot: (value: string) => void;
  meetingId: string;
  setMeetingId: (value: string) => void;
  initialize: (sourceMedia: string, controlledArtifact: string) => Promise<boolean>;
};

const formatDuration = (durationMs: number) => {
  const seconds = Math.max(0, Math.round(durationMs / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
};

function friendlyPreparationError(error: unknown, t: (key: string) => string) {
  const detail = String(error);
  const lower = detail.toLowerCase();
  if (lower.includes('dataset root does not exist')) return t('Dataset Root does not exist');
  if (lower.includes('source media does not exist')) return t('Source Media does not exist');
  if (lower.includes('does not match selected meeting')) return t('Production Artifact does not match selected meeting');
  if (lower.includes('already been initialized')) return t('Existing annotation project detected');
  if (lower.includes('annotation windows already exist')) return t('Existing annotation windows detected');
  if (lower.includes('differs from selected artifact')) return t('Production Artifact already exists with different content');
  if (lower.includes('audio decode failed')) return t('Source Media could not be decoded');
  if (lower.includes('candidate')) return t('Short-turn candidate extraction failed');
  if (lower.includes('window generation') || lower.includes('publish prepared')) return t('Annotation window generation failed');
  return detail;
}

export function AnnotationProjectSetup({
  datasetRoot,
  setDatasetRoot,
  meetingId,
  setMeetingId,
  initialize,
}: Props) {
  const { t } = useUiTranslation();
  const [sourceMedia, setSourceMedia] = useState('');
  const [artifactPath, setArtifactPath] = useState('');
  const [inspection, setInspection] = useState<ProductionArtifactInspection | null>(null);
  const [prepared, setPrepared] = useState<AnnotationPreparationResult | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [initializing, setInitializing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const resetPreparation = () => setPrepared(null);
  const inspectArtifact = async (path: string) => {
    const selected = path.trim();
    if (!selected) {
      setInspection(null);
      setMeetingId('');
      return;
    }
    setInspecting(true);
    setError(null);
    setInspection(null);
    setMeetingId('');
    resetPreparation();
    try {
      const value = await invoke<ProductionArtifactInspection>(
        'api_inspect_short_turn_production_artifact',
        { path: selected },
      );
      setInspection(value);
      setMeetingId(value.meetingId);
    } catch (cause) {
      console.error('api_inspect_short_turn_production_artifact failed', cause);
      const message = `${t('Production Artifact is invalid')}: ${String(cause)}`;
      setError(message);
      toast.error(message);
    } finally {
      setInspecting(false);
    }
  };

  const pickDatasetRoot = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, title: t('Select Dataset Root') });
      if (typeof selected === 'string') {
        setDatasetRoot(selected);
        resetPreparation();
        setError(null);
      }
    } catch (cause) {
      console.error('Dataset Root picker failed', cause);
      toast.error(String(cause));
    }
  };

  const pickSourceMedia = async () => {
    try {
      const selected = await open({
        multiple: false,
        title: t('Select Source Media'),
        filters: [{ name: t('Source Media'), extensions: ['wav', 'mp3', 'm4a', 'mp4', 'webm'] }],
      });
      if (typeof selected === 'string') {
        setSourceMedia(selected);
        resetPreparation();
        setError(null);
      }
    } catch (cause) {
      console.error('Source Media picker failed', cause);
      toast.error(String(cause));
    }
  };

  const pickArtifact = async () => {
    try {
      const selected = await open({
        multiple: false,
        title: t('Select Production Artifact'),
        filters: [{ name: t('Production Artifact'), extensions: ['json'] }],
      });
      if (typeof selected === 'string') {
        setArtifactPath(selected);
        await inspectArtifact(selected);
      }
    } catch (cause) {
      console.error('Production Artifact picker failed', cause);
      toast.error(String(cause));
    }
  };

  const prepare = async () => {
    if (!inspection || preparing) return;
    setPreparing(true);
    setError(null);
    try {
      const result = await invoke<AnnotationPreparationResult>(
        'api_prepare_short_turn_annotation_windows',
        {
          request: {
            datasetRoot: datasetRoot.trim(),
            sourceMedia: sourceMedia.trim(),
            productionArtifact: artifactPath.trim(),
            meetingId: inspection.meetingId,
          },
        },
      );
      setPrepared(result);
      toast.success(t('Annotation data prepared'));
    } catch (cause) {
      console.error('api_prepare_short_turn_annotation_windows failed', cause);
      const message = friendlyPreparationError(cause, t);
      setError(message);
      toast.error(message);
    } finally {
      setPreparing(false);
    }
  };

  const initializePrepared = async () => {
    if (!prepared || initializing) return;
    setInitializing(true);
    setError(null);
    const succeeded = await initialize(sourceMedia, prepared.controlledProductionArtifact);
    if (!succeeded) setError(t('Initialize failed'));
    setInitializing(false);
  };

  const inputsComplete = Boolean(
    datasetRoot.trim() && sourceMedia.trim() && artifactPath.trim() && inspection
      && meetingId === inspection.meetingId,
  );
  const steps = [
    [t('Select inputs'), inputsComplete],
    [t('Prepare annotation data'), Boolean(prepared)],
    [t('Initialize annotation project'), false],
    [t('Blind annotation'), false],
  ] as const;

  return (
    <section className="mx-auto max-w-3xl rounded-xl border border-primary/40 bg-card p-6 shadow-sm">
      <h2 className="text-lg font-semibold">{t('Prepare Annotation Project')}</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        {t('Select the dataset root and frozen meeting inputs. HuiTrace prepares both annotation passes before initialization.')}
      </p>
      <ol className="mt-4 grid gap-2 sm:grid-cols-4">
        {steps.map(([label, complete], index) => (
          <li key={label} className={`rounded-lg border px-3 py-2 text-xs ${complete ? 'border-success/50 bg-success/10 text-success' : 'border-border bg-muted/30 text-muted-foreground'}`}>
            <span className="block font-semibold">{t('Step')} {index + 1}</span>
            <span className="mt-0.5 flex items-center gap-1">{complete ? <Check size={13} aria-hidden="true" /> : <span aria-hidden="true">○</span>}{label}</span>
          </li>
        ))}
      </ol>

      <div className="mt-5 space-y-4">
        <PathField label={t('Dataset Root')} value={datasetRoot} onChange={value => { setDatasetRoot(value); resetPreparation(); }} onPick={pickDatasetRoot} buttonLabel={t('Select folder')} icon={<FolderOpen size={16} aria-hidden="true" />} />
        <PathField label={t('Source Media')} value={sourceMedia} onChange={value => { setSourceMedia(value); resetPreparation(); }} onPick={pickSourceMedia} buttonLabel={t('Select file')} icon={<FileAudio size={16} aria-hidden="true" />} />
        <PathField label={t('Production Artifact')} value={artifactPath} onChange={value => { setArtifactPath(value); setInspection(null); setMeetingId(''); resetPreparation(); }} onBlur={() => void inspectArtifact(artifactPath)} onPick={pickArtifact} buttonLabel={t('Select file')} icon={<FileJson size={16} aria-hidden="true" />} />
        <label className="block text-sm">
          {t('Meeting ID')}
          <input readOnly value={meetingId} placeholder={t('Select a valid Production Artifact first')} className="mt-1 w-full rounded border border-input bg-muted/50 p-2 font-mono text-sm text-foreground" />
          <span className="mt-1 block text-xs text-muted-foreground">{t('Meeting ID is read from Production Artifact')}</span>
        </label>
      </div>

      {inspecting && <p aria-live="polite" className="mt-4 flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="animate-spin" size={16} />{t('Inspecting Production Artifact…')}</p>}
      {inspection && (
        <section aria-label={t('Artifact details')} className="mt-4 rounded-lg border border-border bg-muted/30 p-4 text-sm">
          <h3 className="font-semibold">{t('Artifact details')}</h3>
          <dl className="mt-2 grid gap-x-5 gap-y-1 sm:grid-cols-2">
            <Detail label={t('Artifact ID')} value={inspection.artifactId} />
            <Detail label={t('Transcription run ID')} value={inspection.transcriptionRunId} />
            <Detail label={t('Schema')} value={`v${inspection.schemaVersion}`} />
            <Detail label={t('Duration')} value={formatDuration(inspection.durationMs)} />
            <Detail label="ASR" value={`${inspection.asrBackend} / ${inspection.asrModel}`} />
            <Detail label={t('Diarization')} value={`${inspection.diarizationBackend} / ${inspection.diarizationModel}`} />
            <Detail label={t('Transcript rows')} value={String(inspection.transcriptCount)} />
            <Detail label={t('Diarizer turns')} value={String(inspection.diarizerTurnCount)} />
            <Detail label={t('VAD events')} value={String(inspection.vadEventCount)} />
          </dl>
          {inspection.vadEventCount === 0 && <p className="mt-3 flex gap-2 rounded bg-warning/10 p-2 text-warning"><AlertTriangle className="shrink-0" size={16} />{t('No VAD events were found in this Production Artifact')}</p>}
        </section>
      )}

      {error && <p role="alert" className="mt-4 rounded border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{error}</p>}

      {!prepared ? (
        <button disabled={!inputsComplete || inspecting || preparing} onClick={() => void prepare()} className="mt-5 inline-flex min-h-10 items-center gap-2 rounded bg-primary px-4 py-2 font-semibold text-primary-foreground disabled:opacity-40">
          {preparing && <Loader2 className="animate-spin" size={16} aria-hidden="true" />}
          {preparing ? t('Preparing annotation data…') : t('Prepare annotation data')}
        </button>
      ) : (
        <section className="mt-5 rounded-lg border border-success/50 bg-success/10 p-4">
          <h3 className="flex items-center gap-2 font-semibold text-success"><Check size={18} />{t('Annotation data prepared')}</h3>
          <dl className="mt-2 grid gap-2 text-sm sm:grid-cols-3">
            <Detail label={t('Blind windows')} value={String(prepared.blindWindowCount)} />
            <Detail label={t('Review windows')} value={String(prepared.reviewWindowCount)} />
            <Detail label={t('Candidate suggestions')} value={String(prepared.candidateCount)} />
          </dl>
          <p className="mt-2 text-xs text-muted-foreground">{t('Production Artifact is bound to this meeting')}</p>
          <button disabled={initializing} onClick={() => void initializePrepared()} className="mt-4 inline-flex min-h-10 items-center gap-2 rounded bg-primary px-4 py-2 font-semibold text-primary-foreground disabled:opacity-40">
            {initializing && <Loader2 className="animate-spin" size={16} aria-hidden="true" />}
            {initializing ? t('Initializing annotation project…') : t('Initialize and start Blind annotation')}
          </button>
        </section>
      )}
      <p className="mt-4 rounded bg-warning/10 p-3 text-sm text-warning"><AlertTriangle className="mr-1 inline" size={16} />{t('Review stays locked until every expected Blind window is complete.')}</p>
    </section>
  );
}

function PathField({ label, value, onChange, onBlur, onPick, buttonLabel, icon }: {
  label: string; value: string; onChange: (value: string) => void; onBlur?: () => void;
  onPick: () => void; buttonLabel: string; icon: ReactNode;
}) {
  const inputId = useId();
  return <div className="text-sm"><label htmlFor={inputId}>{label}</label><div className="mt-1 flex gap-2"><input id={inputId} value={value} onChange={event => onChange(event.target.value)} onBlur={onBlur} className="min-w-0 flex-1 rounded border border-input bg-background p-2 font-mono text-sm text-foreground" /><button type="button" onClick={onPick} className="inline-flex min-h-10 shrink-0 items-center gap-2 rounded border border-border bg-background px-3 text-sm font-medium text-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{icon}{buttonLabel}</button></div></div>;
}

function Detail({ label, value }: { label: string; value: string }) {
  return <div className="flex min-w-0 justify-between gap-3"><dt className="text-muted-foreground">{label}</dt><dd className="truncate font-mono text-foreground" title={value}>{value}</dd></div>;
}
