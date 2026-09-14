"use client";

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useRouter } from 'next/navigation';
import { ChevronDown, ExternalLink, FileJson, FlaskConical, Loader2 } from 'lucide-react';
import { toast } from 'sonner';

import type { TalkTimeState } from '@/components/report/SpeakerTurns';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useUiTranslation } from '@/i18n/client';
import { isShortTurnAnnotationEnabled } from '@/lib/shortTurnAnnotationFeature';

export function artifactExportBlockedReason(
  meetingId: string | undefined,
  state: TalkTimeState | null,
  translate: (key: string) => string,
) {
  if (!meetingId) return translate('Meeting ID is unavailable.');
  if (state?.kind === 'done' && (state.job === 'queued' || state.job === 'running')) {
    return translate('Speaker analysis is still running.');
  }
  if (state?.kind === 'done') return null;
  if (state?.kind === 'queued' || state?.kind === 'running') {
    return translate('Speaker analysis is still running.');
  }
  if (!state) return translate('Speaker analysis status is unavailable.');
  return translate('Complete speaker analysis first.');
}

export function EvaluationToolsMenu({
  meetingId,
  diarizationState,
}: {
  meetingId?: string;
  diarizationState: TalkTimeState | null;
}) {
  const { t } = useUiTranslation();
  const router = useRouter();
  const [exporting, setExporting] = useState(false);

  if (!isShortTurnAnnotationEnabled()) return null;

  const blockedReason = artifactExportBlockedReason(meetingId, diarizationState, t);
  const exportArtifact = async () => {
    if (blockedReason || exporting || !meetingId) return;
    setExporting(true);
    try {
      const path = await invoke<string | null>('api_export_short_turn_production_artifact', {
        meetingId,
      });
      // `null` is an ordinary native Save dialog cancellation.
      if (path) toast.success(t('Production Artifact exported.'));
    } catch (error) {
      console.error('api_export_short_turn_production_artifact failed', error);
      const detail = String(error);
      if (detail.toLowerCase().includes('no completed production short-turn snapshot')) {
        toast.error(t('Failed to export Production Artifact'), {
          description: t('This meeting does not have a complete evaluation snapshot yet. Complete speaker analysis before exporting.'),
        });
      } else {
        toast.error(t('Failed to export Production Artifact'), { description: detail });
      }
    } finally {
      setExporting(false);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="text-muted-foreground hover:text-foreground"
          aria-label={t('Evaluation Tools')}
        >
          <FlaskConical aria-hidden="true" className="h-4 w-4" />
          <span className="hidden lg:inline">{t('Evaluation')}</span>
          <ChevronDown aria-hidden="true" className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-80">
        <DropdownMenuLabel>{t('Evaluation Tools')}</DropdownMenuLabel>
        <div className="px-2 pb-2 text-xs leading-5 text-muted-foreground">
          <ol className="list-inside list-decimal">
            <li>{t('Complete speaker analysis')}</li>
            <li>{t('Export Production Artifact')}</li>
            <li>{t('Generate annotation windows')}</li>
            <li>{t('Open Annotation Workspace')}</li>
          </ol>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={Boolean(blockedReason) || exporting}
          onSelect={() => void exportArtifact()}
          title={blockedReason ?? undefined}
          aria-busy={exporting}
          className="items-start py-2"
        >
          {exporting
            ? <Loader2 aria-hidden="true" className="mt-0.5 animate-spin" />
            : <FileJson aria-hidden="true" className="mt-0.5" />}
          <span className="min-w-0">
            <span className="block font-medium">
              {exporting ? t('Exporting Production Artifact…') : t('Export Production Artifact')}
            </span>
            {blockedReason && <span className="block text-xs text-muted-foreground">{blockedReason}</span>}
          </span>
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => router.push(
            meetingId
              ? `/dev/short-turn-annotation?meetingId=${encodeURIComponent(meetingId)}`
              : '/dev/short-turn-annotation',
          )}
          className="items-start py-2"
        >
          <ExternalLink aria-hidden="true" className="mt-0.5" />
          <span className="min-w-0">
            <span className="block font-medium">{t('Open Annotation Workspace')}</span>
            <span className="block text-xs text-muted-foreground">
              {t('Prepare annotation windows before opening the workspace.')}
            </span>
          </span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
