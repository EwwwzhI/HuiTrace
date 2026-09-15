"use client";

import { Transcript, TranscriptSegmentData, UtteranceReconstructionResult } from '@/types';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { TranscriptButtonGroup } from './TranscriptButtonGroup';
import { FileText, ChevronDown, Users } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TOUR_ANCHORS } from '@/lib/tour';
import { TalkTimePanel } from '@/components/report/SpeakerTurns';
import { useDiarization } from '@/hooks/useDiarization';
import { speakerCount } from '@/lib/speakerTurns';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import { EvaluationToolsMenu } from './EvaluationToolsMenu';


interface TranscriptPanelProps {
  transcripts: Transcript[];
  customPrompt: string;
  onPromptChange: (value: string) => void;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  isRecording: boolean;
  disableAutoScroll?: boolean;

  // Optional pagination props (when using virtualization)
  usePagination?: boolean;
  segments?: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;

  // Retranscription props
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;

  // Jump-to-source (BACKLOG C1.6): plumbed from the summary draft review surface
  // down to the virtualized transcript view. Additive; default undefined = today.
  scrollToSegmentId?: string | null;
  scrollNonce?: number;
  onRequestSegment?: (segmentId: string) => void;
  /** Click-to-play: seek meeting audio to a segment's start time. */
  onSeekToTime?: (sec: number) => void;
}

export function TranscriptPanel({
  transcripts,
  customPrompt,
  onPromptChange,
  onCopyTranscript,
  onOpenMeetingFolder,
  isRecording,
  disableAutoScroll = false,
  usePagination = false,
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  scrollToSegmentId,
  scrollNonce,
  onRequestSegment,
  onSeekToTime,
}: TranscriptPanelProps) {
  useUiTranslation();
  // Convert transcripts to segments if pagination is not used but we want virtualization
  const convertedSegments = useMemo<TranscriptSegmentData[]>(() => {
    if (usePagination && segments) {
      return segments;
    }
    // Convert transcripts to segments for virtualization
    return transcripts.map(t => ({
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
      asr_confidence: t.asr_confidence,
      speaker_id: t.speaker_id,
      speaker_confidence: t.speaker_confidence,
      speaker_provisional: t.speaker_provisional,
      speaker_revision: t.speaker_revision,
      segment_kind: t.segment_kind,
      audio_source: t.audio_source,
      speaker_assignment_method: t.speaker_assignment_method,
      speaker_overlap: t.speaker_overlap,
    }));
  }, [transcripts, usePagination, segments]);

  // Speaker separation is a POST-HOC pass over a finished recording (ADR-0034),
  // so it is offered only once recording has stopped. Showing it mid-recording
  // would advertise an action that cannot run yet.
  const diarization = useDiarization(isRecording ? undefined : meetingId);
  const [isSpeakersOpen, setIsSpeakersOpen] = useState(false);
  const [viewMode, setViewMode] = useState<'reconstructed' | 'raw'>('reconstructed');
  const [reconstruction, setReconstruction] = useState<UtteranceReconstructionResult | null>(null);
  const [isReconstructionLoading, setIsReconstructionLoading] = useState(false);
  const speakerPanelId = `speaker-panel-${useId().replace(/:/g, '')}`;

  const loadReconstruction = useCallback(async () => {
    if (!meetingId || isRecording) return;
    setIsReconstructionLoading(true);
    try {
      const result = await invoke<UtteranceReconstructionResult>('api_get_reconstructed_utterances', {
        meetingId,
      });
      if (!result || !Array.isArray(result.utterances)) {
        throw new Error('Invalid utterance reconstruction response');
      }
      setReconstruction(result);
    } catch (error) {
      // Presentation enhancement is fail-open: raw ASR evidence remains usable.
      console.warn('[TranscriptPanel] Utterance reconstruction unavailable; showing raw transcript', error);
      setReconstruction(null);
      setViewMode('raw');
    } finally {
      setIsReconstructionLoading(false);
    }
  }, [isRecording, meetingId]);

  const diarizationSignature = useMemo(
    () => diarization.turns
      .map((turn) => `${turn.start_ms}:${turn.end_ms}:${turn.speaker_key}`)
      .join('|'),
    [diarization.turns]
  );
  const shortTurnSignature = useMemo(
    () => diarization.events
      .map((event) => `${event.id}:${event.revision}:${event.kind}:${event.speaker_key ?? ''}`)
      .join('|'),
    [diarization.events]
  );

  useEffect(() => {
    setReconstruction(null);
    setViewMode('reconstructed');
  }, [meetingId]);

  useEffect(() => {
    void loadReconstruction();
  }, [loadReconstruction, diarizationSignature, shortTurnSignature]);

  const reconstructedSegments = useMemo<TranscriptSegmentData[]>(() => {
    if (!reconstruction) return [];
    const utterances = reconstruction.utterances.map((utterance) => ({
      id: utterance.id,
      timestamp: utterance.start_ms / 1000,
      endTime: utterance.end_ms / 1000,
      text: utterance.text,
      confidence: utterance.mean_asr_confidence ?? undefined,
      asr_confidence: utterance.mean_asr_confidence ?? undefined,
      speaker_id: utterance.speaker_attribution.kind === 'single'
        ? utterance.speaker_attribution.speaker_key
        : undefined,
      segment_kind: 'speech',
      speaker_overlap: utterance.overlap,
      source_chunk_ids: utterance.source_transcript_ids,
      reconstructed: true,
      embedded_events: utterance.embedded_events,
      speaker_attribution: utterance.speaker_attribution,
    }));
    const standaloneEvents = reconstruction.events.map((event) => ({
      id: event.id,
      timestamp: event.start_ms / 1000,
      endTime: event.end_ms / 1000,
      text: event.text,
      confidence: event.confidence ?? undefined,
      asr_confidence: event.confidence ?? undefined,
      speaker_id: event.speaker_attribution.kind === 'single'
        ? event.speaker_attribution.speaker_key
        : undefined,
      segment_kind: event.kind,
      speaker_overlap: event.overlap,
      source_chunk_ids: event.source_transcript_ids,
      reconstructed: true,
      speaker_attribution: event.speaker_attribution,
    }));
    return [...utterances, ...standaloneEvents].sort((left, right) =>
      left.timestamp - right.timestamp || left.id.localeCompare(right.id)
    );
  }, [reconstruction]);

  const showingReconstructed = viewMode === 'reconstructed' && reconstructedSegments.length > 0;
  const displaySegments = showingReconstructed ? reconstructedSegments : convertedSegments;
  const displayCount = showingReconstructed
    ? reconstructedSegments.length
    : usePagination ? (totalCount ?? convertedSegments.length) : (transcripts?.length || 0);

  const handleCopyDisplayedTranscript = useCallback(() => {
    if (!showingReconstructed) {
      onCopyTranscript();
      return;
    }
    const text = displaySegments
      .flatMap((segment) => [segment.text, ...(segment.embedded_events ?? []).map((event) => event.text)])
      .join('\n');
    void navigator.clipboard.writeText(text);
  }, [displaySegments, onCopyTranscript, showingReconstructed]);

  const refreshTranscriptsAndReconstruction = useCallback(async () => {
    await onRefetchTranscripts?.();
    await loadReconstruction();
  }, [loadReconstruction, onRefetchTranscripts]);

  useEffect(() => {
    setIsSpeakersOpen(false);
  }, [meetingId]);

  return (
    // Layout-neutral root: width, borders, and responsive show/hide are owned by
    // the wrapper in page-content.tsx so the split can be rebalanced and made
    // responsive/collapsible without threading layout state through every prop.
    // data-tour: anchor for the first-run product tour (step 1).
    <div data-tour={TOUR_ANCHORS.transcriptPanel} className="flex w-full h-full min-h-0 min-w-0 overflow-hidden bg-background flex-col relative">
      {/* Panel toolbar: identity (icon + title + segment count) on the left,
          transcript actions on the right — replaces the floating centered row. */}
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2">
          <span className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-accent text-accent-foreground">
            <FileText className="h-3.5 w-3.5" aria-hidden />
          </span>
          <h2 className="truncate text-sm font-semibold text-foreground">{translateUI("Transcript")}</h2>
          <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
            {displayCount}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {!isRecording && meetingId && convertedSegments.length > 0 && (
            <div className="mr-1 flex rounded-md bg-muted p-0.5" aria-label={translateUI('Transcript view')}>
              <button
                type="button"
                aria-pressed={showingReconstructed}
                disabled={isReconstructionLoading || !reconstruction?.utterances.length}
                onClick={() => setViewMode('reconstructed')}
                className={`rounded px-2 py-1 text-xs transition-colors ${showingReconstructed ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'} disabled:opacity-50`}
              >
                {translateUI('Reordered')}
              </button>
              <button
                type="button"
                aria-pressed={!showingReconstructed}
                onClick={() => setViewMode('raw')}
                className={`rounded px-2 py-1 text-xs transition-colors ${!showingReconstructed ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
              >
                {translateUI('Raw')}
              </button>
            </div>
          )}
          <EvaluationToolsMenu meetingId={meetingId} diarizationState={diarization.state} />
          <TranscriptButtonGroup
            transcriptCount={displayCount}
            onCopyTranscript={handleCopyDisplayedTranscript}
            onOpenMeetingFolder={onOpenMeetingFolder}
            meetingId={meetingId}
            meetingFolderPath={meetingFolderPath}
            onRefetchTranscripts={refreshTranscriptsAndReconstruction}
          />
        </div>
      </div>

      {/* Who spoke, and for how long. Absent entirely while recording, and while
          we have not yet been able to ask -- an empty space says nothing, which
          is the honest thing to say when we do not know. */}
      {(diarization.state || diarization.error) && (
        <div className="v2-speaker-disclosure min-h-0 max-h-[35%] shrink-0 overflow-y-auto border-b border-border">
          <button
            id={`${speakerPanelId}-trigger`}
            type="button"
            aria-expanded={isSpeakersOpen}
            aria-controls={speakerPanelId}
            onClick={() => setIsSpeakersOpen((open) => !open)}
            className="v2-speaker-trigger sticky top-0 z-10 flex w-full items-center gap-2 bg-background px-4 py-3 text-left text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          >
            <Users className="h-4 w-4 text-muted-foreground" aria-hidden /> {translateUI("Speakers")} {diarization.turns.length > 0 && <span className="text-xs text-muted-foreground">{speakerCount(diarization.turns)} {translateUI("· best-effort estimate")}</span>}
            {diarization.busy && <span role="status" className="text-xs text-muted-foreground">{translateUI("Processing…")}</span>}
            {diarization.error && <span className="text-xs text-destructive">{translateUI("Needs attention")}</span>}
            <ChevronDown className="v2-speaker-chevron ml-auto h-4 w-4 shrink-0" aria-hidden />
          </button>
          <div id={speakerPanelId} role="region" aria-labelledby={`${speakerPanelId}-trigger`} className={`v2-speaker-content ${isSpeakersOpen ? 'is-open' : ''}`}>
          <div className="min-h-0 overflow-hidden px-4 pb-3">
          {diarization.state && (
            <TalkTimePanel
              state={diarization.state}
              onRun={diarization.run}
              onGetModels={diarization.downloadModels}
              busy={diarization.busy}
              onRename={diarization.renameSpeaker}
            />
          )}
          {/* Rendered whether or not there is a state. When the very first
              availability query fails the hook has no state to report -- gating
              the error on the state would make the whole feature vanish with no
              explanation, which reads as "this meeting has no speakers" rather
              than "we could not find out". */}
          {diarization.error && (
            <p className="text-xs text-destructive" role="status"> {translateUI("Speakers could not be checked:")} {diarization.error}{' '}
              <button
                type="button"
                onClick={diarization.refresh}
                className="underline underline-offset-2 hover:no-underline"
              > {translateUI("Try again")} </button>
            </p>
          )}
          </div>
          </div>
        </div>
      )}

      {/* Transcript content - use virtualized view for better performance */}
      <div className="flex-1 min-h-0 overflow-hidden pb-4">
        <VirtualizedTranscriptView
          segments={displaySegments}
          speakerTurns={diarization.turns}
          shortTurnEvents={diarization.events}
          onAssignSpeaker={showingReconstructed ? undefined : async (transcriptId, speakerKey) => { await diarization.assignTranscriptSpeaker(transcriptId, speakerKey); await onRefetchTranscripts?.(); await diarization.refresh(); await loadReconstruction(); }}
          onAssignShortTurnEventSpeaker={diarization.assignShortTurnEventSpeaker}
          isRecording={isRecording}
          isPaused={false}
          isProcessing={false}
          isStopping={false}
          enableStreaming={false}
          showConfidence={true}
          disableAutoScroll={disableAutoScroll}
          hasMore={showingReconstructed ? false : hasMore}
          isLoadingMore={isLoadingMore}
          totalCount={showingReconstructed ? reconstructedSegments.length : totalCount}
          loadedCount={showingReconstructed ? reconstructedSegments.length : loadedCount}
          onLoadMore={showingReconstructed ? undefined : onLoadMore}
          scrollToSegmentId={scrollToSegmentId}
          scrollNonce={scrollNonce}
          onRequestSegment={onRequestSegment}
          onSeekToTime={onSeekToTime}
        />
      </div>

      {/* Custom prompt input at bottom of transcript section */}
      {!isRecording && displaySegments.length > 0 && (
        <div className="shrink-0 border-t border-border p-3">
          <textarea
            placeholder={translateUI("Add context for the AI summary — people involved, meeting overview, objective…")}
            className="h-[72px] w-full resize-none rounded-lg border border-input bg-card px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/70 shadow-sm focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring"
            value={customPrompt}
            onChange={(e) => onPromptChange(e.target.value)}
          />
        </div>
      )}
    </div>
  );
}
