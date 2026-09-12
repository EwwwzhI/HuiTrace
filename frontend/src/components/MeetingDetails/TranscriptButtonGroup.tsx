"use client";

import { useState, useCallback } from 'react';
import { Button } from '@/components/ui/button';
import { Copy, FolderOpen, RefreshCw } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { RetranscribeDialog } from './RetranscribeDialog';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';




interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  useUiTranslation();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  return (
    // Compact toolbar cluster; the parent header owns alignment (right side).
    <div className="flex shrink-0 items-center gap-1">
      <Button
        variant="ghost"
        size="sm"
        className="text-muted-foreground hover:text-foreground"
        onClick={() => {
          Analytics.trackButtonClick('copy_transcript', 'meeting_details');
          onCopyTranscript();
        }}
        disabled={transcriptCount === 0}
        title={transcriptCount === 0 ? translateUI("No transcript available") : translateUI("Copy Transcript")}
      >
        <Copy className="h-4 w-4" />
        <span className="hidden lg:inline">{translateUI("Copy")}</span>
      </Button>

      <Button
        size="sm"
        variant="ghost"
        className="text-muted-foreground hover:text-foreground"
        onClick={() => {
          Analytics.trackButtonClick('open_recording_folder', 'meeting_details');
          onOpenMeetingFolder();
        }}
        title={translateUI("Open Audio Folder")}
      >
        <FolderOpen className="h-4 w-4" />
        <span className="hidden lg:inline">{translateUI("Audio")}</span>
      </Button>

      {meetingId && meetingFolderPath && (
        <Button
          size="sm"
          variant="ghost"
          className="text-primary hover:bg-accent hover:text-primary"
          onClick={() => {
            Analytics.trackButtonClick('enhance_transcript', 'meeting_details');
            setShowRetranscribeDialog(true);
          }}
          title={translateUI("Retranscribe to enhance your recorded audio")}
        >
          <RefreshCw className="h-4 w-4" />
          <span className="hidden lg:inline">{translateUI("Enhance")}</span>
        </Button>
      )}

      {meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
