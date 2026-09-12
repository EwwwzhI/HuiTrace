"use client";
import { useState, useEffect, useRef } from 'react';
import dynamic from 'next/dynamic';
import { Summary, SummaryResponse } from '@/types';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import Analytics from '@/lib/analytics';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { TranscriptPanel } from '@/components/MeetingDetails/TranscriptPanel';
import { NotePanelSkeleton } from './meeting-details-skeleton';
import paneStyles from '@/components/MeetingDetails/CollapsibleSummaryPane.module.css';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { FileText, Sparkles, PanelRightOpen } from 'lucide-react';
import { ReportHeader } from '@/components/report/ReportHeader';
import { TopicsTimeline } from '@/components/report/TopicsTimeline';
import { PlaybackBar, PlaybackBarHandle } from '@/components/report/PlaybackBar';


const SummaryPanel = dynamic(() => import('@/components/MeetingDetails/SummaryPanel').then(module => module.SummaryPanel), {
  loading: () => <NotePanelSkeleton />,
  ssr: false,
});

// Custom hooks
import { useMeetingData } from '@/hooks/meeting-details/useMeetingData';
import { useSummaryGeneration } from '@/hooks/meeting-details/useSummaryGeneration';
import { useTemplates } from '@/hooks/meeting-details/useTemplates';
import { useCopyOperations } from '@/hooks/meeting-details/useCopyOperations';
import { useMeetingOperations } from '@/hooks/meeting-details/useMeetingOperations';
import { useConfig } from '@/contexts/ConfigContext';
import { useTour } from '@/components/tour';
import { TOUR_ANCHORS } from '@/lib/tour';

export default function PageContent({
  meeting,
  summaryData,
  isSummaryLoading = false,
  initialSegmentId,
  initialJumpId,
  shouldAutoGenerate = false,
  onAutoGenerateComplete,
  onMeetingUpdated,
  onRefetchTranscripts,
  // Pagination props for efficient transcript loading
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
}: {
  meeting: any;
  summaryData: Summary | null;
  isSummaryLoading?: boolean;
  initialSegmentId?: string | null;
  initialJumpId?: string | null;
  shouldAutoGenerate?: boolean;
  onAutoGenerateComplete?: () => void;
  onMeetingUpdated?: () => Promise<void>;
  onRefetchTranscripts?: () => Promise<void>;
  // Pagination props
  segments?: any[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
}) {
  useUiTranslation();
  console.log('📄 PAGE CONTENT: Initializing with data:', {
    meetingId: meeting.id,
    summaryDataKeys: summaryData ? Object.keys(summaryData) : null,
    transcriptsCount: meeting.transcripts?.length
  });

  // State
  const [customPrompt, setCustomPrompt] = useState<string>('');
  const [isRecording] = useState(false);
  const [summaryResponse] = useState<SummaryResponse | null>(null);

  // BACKLOG C1.6 — jump-to-source: the source_chunk_id the review surface asked
  // to reveal, plus a nonce so repeat clicks on the same source re-trigger the
  // scroll+flash. `source_chunk_id` shares the transcripts-table row id space,
  // so it is used directly as the target segment id.
  const [scrollToSegmentId, setScrollToSegmentId] = useState<string | null>(null);
  const [scrollNonce, setScrollNonce] = useState(0);

  // Meeting-details split layout (frontend-only rebalance):
  //  - Desktop (md+): transcript (primary) grows to fill; summary is capped and
  //    can be COLLAPSED so the transcript reclaims the full width.
  //  - Narrow (< md): the two panels are switched via an in-page tab bar so the
  //    transcript (primary content) is always reachable — it used to be hidden.
  // Both panels stay mounted at all times (CSS show/hide, never unmount) so the
  // BlockNote editor / draft-review state and the transcript scroll position
  // survive collapsing and tab switches.
  const [isSummaryCollapsed, setIsSummaryCollapsed] = useState(false);
  const [mobileTab, setMobileTab] = useState<'transcript' | 'summary'>('transcript');
  const [summaryReady, setSummaryReady] = useState(false);
  useEffect(() => {
    // Give the transcript a paint before mounting the editor and its toolbars.
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => setSummaryReady(true));
    });
    return () => { cancelAnimationFrame(firstFrame); cancelAnimationFrame(secondFrame); };
  }, []);

  // Product-tour step 2 points at a source-linked summary block. Reveal the
  // summary before the coach-mark looks for it: expand it if collapsed and, on
  // narrow windows, switch to the summary tab. Without this the block can be
  // display:hidden, and the step would land centered instead of on it.
  const { activeAnchor } = useTour();
  useEffect(() => {
    if (activeAnchor === TOUR_ANCHORS.summaryApproveBlock) {
      setIsSummaryCollapsed(false);
      setMobileTab('summary');
    } else if (activeAnchor === TOUR_ANCHORS.transcriptPanel) {
      // On narrow windows the two panes are tabbed; make sure the transcript is
      // the visible one for step 1 so Back from step 2 lands on it, not centered.
      setMobileTab('transcript');
    }
  }, [activeAnchor]);

  // Evidence-search deep links reuse the existing C1.6 jump-to-source path.
  // Reset the target when the meeting changes so a prior search hit cannot be
  // retried against a different meeting; on narrow screens always reveal the
  // transcript tab before the virtualized view scrolls to the source.
  useEffect(() => {
    setScrollToSegmentId(initialSegmentId ?? null);
    if (initialSegmentId) {
      setMobileTab('transcript');
      setScrollNonce((nonce) => nonce + 1);
    }
  }, [initialSegmentId, initialJumpId, meeting.id]);

  // Ref to store the modal open function from SummaryGeneratorButtonGroup
  const openModelSettingsRef = useRef<(() => void) | null>(null);

  // Phase D (playback sync): imperative handle into the meeting's audio bar so
  // transcript timestamps and chapter blocks can click-to-play.
  const playbackRef = useRef<PlaybackBarHandle>(null);
  const handleSeekToTime = (sec: number) => playbackRef.current?.seekTo(sec);

  // Sidebar context
  const { serverAddress } = useSidebar();

  // Get model config + beta features from ConfigContext
  const { modelConfig, setModelConfig } = useConfig();

  // Custom hooks
  const meetingData = useMeetingData({ meeting, summaryData, onMeetingUpdated });
  const templates = useTemplates();

  // Callback to register the modal open function
  const handleRegisterModalOpen = (openFn: () => void) => {
    console.log('📝 Registering modal open function in PageContent');
    openModelSettingsRef.current = openFn;
  };

  // Callback to trigger modal open (called from error handler)
  const handleOpenModelSettings = () => {
    console.log('🔔 Opening model settings from PageContent');
    if (openModelSettingsRef.current) {
      openModelSettingsRef.current();
    } else {
      console.warn('⚠️ Modal open function not yet registered');
    }
  };

  // Only evidence-linked rows enter the HITL review surface. Pre-v1.0.4 summaries
  // remain visible through an explicitly unverified, read-only upgrade view.
  const structuredEnabled = meetingData.hasSummaryDraft;

  // BACKLOG C1.6 — jump from a draft block/action item to its transcript segment.
  const handleJumpToSource = (sourceChunkId: string) => {
    setScrollToSegmentId(sourceChunkId);
    setScrollNonce((n) => n + 1);
    // On the narrow layout the transcript pane is mounted but hidden, so
    // scrolling it would happen out of sight: opening a source has to reveal
    // the transcript too, or the source control silently does nothing. No-op
    // on desktop, where both panes are visible.
    setMobileTab('transcript');
  };

  // The target segment isn't in the loaded page: pull the next page so the
  // transcript view can retry the scroll once it arrives.
  const handleRequestSegment = () => {
    if (hasMore && !isLoadingMore) {
      onLoadMore?.();
    }
  };

  // Save model config to backend database and sync via event
  const handleSaveModelConfig = async (config?: ModelConfig) => {
    if (!config) return;
    try {
      await invoke('api_save_model_config', {
        provider: config.provider,
        model: config.model,
        whisperModel: config.whisperModel,
        apiKey: config.apiKey ?? null,
        ollamaEndpoint: config.ollamaEndpoint ?? null,
      });

      // Emit event so ConfigContext and other listeners stay in sync
      const { emit } = await import('@tauri-apps/api/event');
      await emit('model-config-updated', config);

      toast.success(translateUI("Model settings saved successfully"));
    } catch (error) {
      console.error('Failed to save model config:', error);
      toast.error(translateUI("Failed to save model settings"));
    }
  };

  const summaryGeneration = useSummaryGeneration({
    meeting,
    transcripts: meetingData.transcripts,
    modelConfig: modelConfig,
    isModelConfigLoading: false, // ConfigContext loads on mount
    selectedTemplate: templates.selectedTemplate,
    onMeetingUpdated,
    updateMeetingTitle: meetingData.updateMeetingTitle,
    setAiSummary: meetingData.setAiSummary,
    onOpenModelSettings: handleOpenModelSettings,
    // Rust enforces structured drafts; this compatibility field stays true while
    // older clients and generated bindings still carry it.
    structuredSummaries: true,
    onStructuredGenerated: meetingData.refetchDraft,
  });

  const copyOperations = useCopyOperations({
    meeting,
    meetingTitle: meetingData.meetingTitle,
  });

  const meetingOperations = useMeetingOperations({
    meeting,
  });

  // Track page view
  useEffect(() => {
    Analytics.trackPageView('meeting_details');
  }, []);

  // Auto-generate summary when flag is set
  useEffect(() => {
    let cancelled = false;

    const autoGenerate = async () => {
      if (shouldAutoGenerate && meetingData.transcripts.length > 0 && !cancelled) {
        console.log(`🤖 Auto-generating summary with ${modelConfig.provider}/${modelConfig.model}...`);
        await summaryGeneration.handleGenerateSummary('');

        // Notify parent that auto-generation is complete (only if not cancelled)
        if (onAutoGenerateComplete && !cancelled) {
          onAutoGenerateComplete();
        }
      }
    };

    autoGenerate();

    // Cleanup: cancel if component unmounts or meeting changes
    return () => {
      cancelled = true;
    };
  }, [shouldAutoGenerate, meeting.id]); // Re-run if meeting changes

  return (
    <div
      className={`ink-meeting flex flex-col h-screen bg-background ${paneStyles.root}`}
    >
      {/* read.ai-style report header: title, date/duration meta, and an on-device
          overview-metrics strip computed from the local transcript. */}
      <ReportHeader
        title={meetingData.meetingTitle}
        createdAt={meeting.created_at}
        transcripts={meetingData.transcripts}
      />

      {/* On-device chapters strip (pause-based, deterministic). Clicking a chapter
          jumps the transcript to its first segment via the C1.6 mechanism. Renders
          nothing for short/gap-less meetings. */}
      <TopicsTimeline
        transcripts={meetingData.transcripts}
        onJumpToSegment={(segmentId, startSec) => {
          handleJumpToSource(segmentId);
          handleSeekToTime(startSec);
        }}
      />

      {/* Local/imported audio playback; transcript timestamps + chapters seek into it. */}
      <PlaybackBar ref={playbackRef} meetingId={meeting.id} />

      {/* Narrow-screen (< md) tab bar: switches which panel is visible so the
          transcript (primary content) is reachable on mobile/tablet. Hidden on
          md+ where both panels sit side by side. */}
      <div className={`${paneStyles.tabs} items-center gap-2 px-3 py-2 border-b border-border bg-card`}>
        <button
          type="button"
          onClick={() => setMobileTab('transcript')}
          aria-pressed={mobileTab === 'transcript'}
          className={`flex-1 inline-flex items-center justify-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
            mobileTab === 'transcript'
              ? 'bg-accent text-primary border border-primary/20'
              : 'text-muted-foreground hover:bg-muted border border-transparent'
          }`}
        >
          <FileText size={16} /> {translateUI("Transcript")} </button>
        <button
          type="button"
          onClick={() => setMobileTab('summary')}
          aria-pressed={mobileTab === 'summary'}
          className={`flex-1 inline-flex items-center justify-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
            mobileTab === 'summary'
              ? 'bg-accent text-primary border border-primary/20'
              : 'text-muted-foreground hover:bg-muted border border-transparent'
          }`}
        >
          <Sparkles size={16} /> {translateUI("Summary")} </button>
      </div>

      <div className={`v2-report-panels flex flex-1 min-h-0 overflow-hidden ${paneStyles.layout}`}>
        {/* Transcript wrapper — PRIMARY content.
            - Mobile: full width, visible only when its tab is active.
            - md+: always visible and grows to fill (flex-1), so it is never the
              cramped panel and it reclaims space when the summary collapses.
            The right border only shows on md+ when the summary is visible. */}
        <div
          data-active={mobileTab === 'transcript'}
          className={paneStyles.transcript}
        >
          <TranscriptPanel
          transcripts={meetingData.transcripts}
          customPrompt={customPrompt}
          onPromptChange={setCustomPrompt}
          onCopyTranscript={copyOperations.handleCopyTranscript}
          onOpenMeetingFolder={meetingOperations.handleOpenMeetingFolder}
          isRecording={isRecording}
          disableAutoScroll={true}
          // Pagination props for efficient loading
          usePagination={true}
          segments={segments}
          hasMore={hasMore}
          isLoadingMore={isLoadingMore}
          totalCount={totalCount}
          loadedCount={loadedCount}
          onLoadMore={onLoadMore}
          // Retranscription props
          meetingId={meeting.id}
          meetingFolderPath={meeting.folder_path}
          onRefetchTranscripts={onRefetchTranscripts}
          // Jump-to-source (C1.6)
          scrollToSegmentId={scrollToSegmentId}
          scrollNonce={scrollNonce}
          onRequestSegment={handleRequestSegment}
          onSeekToTime={handleSeekToTime}
          />
        </div>

        {/* Desktop: animate the viewport to a 44px rail while keeping the editor mounted and at its expanded width. Mobile keeps the existing full-width tabs. */}
        <div
          data-collapsed={isSummaryCollapsed}
          data-active={mobileTab === 'summary'}
          className={paneStyles.pane}
        >
          <div id="meeting-summary-content" className={paneStyles.content}>
          {isSummaryLoading || !summaryReady ? (
            <NotePanelSkeleton />
          ) : <SummaryPanel
          meeting={meeting}
          meetingTitle={meetingData.meetingTitle}
          onTitleChange={meetingData.handleTitleChange}
          isEditingTitle={meetingData.isEditingTitle}
          onStartEditTitle={() => meetingData.setIsEditingTitle(true)}
          onFinishEditTitle={() => meetingData.setIsEditingTitle(false)}
          summaryRef={meetingData.blockNoteSummaryRef}
          aiSummary={meetingData.aiSummary}
          summaryStatus={summaryGeneration.summaryStatus}
          transcripts={meetingData.transcripts}
          modelConfig={modelConfig}
          setModelConfig={setModelConfig}
          onSaveModelConfig={handleSaveModelConfig}
          onGenerateSummary={summaryGeneration.handleGenerateSummary}
          onStopGeneration={summaryGeneration.handleStopGeneration}
          customPrompt={customPrompt}
          summaryResponse={summaryResponse}
          onSaveSummary={meetingData.handleSaveSummary}
          onSummaryChange={meetingData.handleSummaryChange}
          onDirtyChange={meetingData.setIsSummaryDirty}
          summaryError={summaryGeneration.summaryError}
          onRegenerateSummary={summaryGeneration.handleRegenerateSummary}
          getSummaryStatusMessage={summaryGeneration.getSummaryStatusMessage}
          availableTemplates={templates.availableTemplates}
          selectedTemplate={templates.selectedTemplate}
          onTemplateSelect={templates.handleTemplateSelection}
          isModelConfigLoading={false}
          onOpenModelSettings={handleRegisterModalOpen}
          // Source-linked structured draft review (C1.6)
          structuredEnabled={structuredEnabled}
          draftResponse={meetingData.draftResponse}
          isDraftLoading={meetingData.isDraftLoading}
          draftError={meetingData.draftError}
          onJumpToSource={handleJumpToSource}
          onSummaryApproved={meetingData.refetchDraft}
          // Desktop collapse control (chevron lives in the summary header).
          showCollapseButton
          onCollapse={() => setIsSummaryCollapsed(true)}
          />}
          </div>
          {/* Keep the rail mounted so rapid reversals continue the transition. */}
          <div className={`${paneStyles.rail} bg-card`}>
            <button
              type="button"
              onClick={() => setIsSummaryCollapsed(false)}
              title={translateUI("Show summary panel")}
              aria-label={translateUI("Show summary panel")}
              aria-expanded={!isSummaryCollapsed}
              aria-controls="meeting-summary-content"
              tabIndex={isSummaryCollapsed ? 0 : -1}
              className="p-2 m-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <PanelRightOpen size={18} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
