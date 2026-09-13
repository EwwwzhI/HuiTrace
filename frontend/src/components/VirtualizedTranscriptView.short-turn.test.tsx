/** @vitest-environment jsdom */

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { VirtualizedTranscriptView } from './VirtualizedTranscriptView';
import { TooltipProvider } from './ui/tooltip';

vi.mock('@/i18n/client', () => ({ useUiTranslation: () => undefined }));
vi.mock('framer-motion', () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
  motion: new Proxy({}, { get: (_target, tag) => tag }),
}));

afterEach(cleanup);

describe('VirtualizedTranscriptView short-turn rendering', () => {
  it.each(['uh', 'hmm', '哦'])('preserves meaningful backchannel %s', (text) => {
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={[{ id: `segment-${text}`, timestamp: 0, endTime: 0.2, text }]}
          disableAutoScroll
          enableStreaming={false}
          showConfidence={false}
        />
      </TooltipProvider>,
    );

    expect(screen.getByText(text)).toBeTruthy();
  });

  it('renders an embedded event as a seekable annotation without inventing text', () => {
    const seek = vi.fn();
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={[{ id: 'long', timestamp: 0, endTime: 5, text: 'A long transcript row' }]}
          shortTurnEvents={[{
            id: 'event-1',
            meeting_id: 'm-1',
            transcript_id: 'long',
            start_ms: 2100,
            end_ms: 2400,
            kind: 'speech',
            kind_confidence: 0.9,
            speaker_key: 'speaker_02',
            speaker_display_name: 'Speaker 2',
            speaker_confidence: 0.8,
            candidate_sources: ['diarizer_turn', 'vad_event'],
            audio_source: 'mixed',
            revision: 1,
            assignment_method: 'short_turn_refinement',
            transcript_aligned: false,
            user_visible: true,
          }]}
          onSeekToTime={seek}
          disableAutoScroll
          enableStreaming={false}
          showConfidence={false}
        />
      </TooltipProvider>,
    );

    expect(screen.getByText('Speaker 2')).toBeTruthy();
    expect(screen.getByText(/Short speech/)).toBeTruthy();
    screen.getByRole('button', { name: 'Play short event' }).click();
    expect(seek).toHaveBeenCalledWith(2.1);
    expect(screen.queryByText('嗯')).toBeNull();
  });

  it('does not render transcript-aligned events twice', () => {
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={[{ id: 'short', timestamp: 0, endTime: 0.3, text: 'hmm' }]}
          shortTurnEvents={[{
            id: 'event-aligned',
            meeting_id: 'm-1',
            transcript_id: 'short',
            start_ms: 0,
            end_ms: 300,
            kind: 'backchannel',
            kind_confidence: 0.9,
            speaker_key: null,
            speaker_display_name: null,
            speaker_confidence: null,
            candidate_sources: ['transcript'],
            audio_source: 'mixed',
            revision: 1,
            assignment_method: 'short_turn_refinement',
            transcript_aligned: true,
            user_visible: true,
          }]}
          disableAutoScroll
          enableStreaming={false}
          showConfidence={false}
        />
      </TooltipProvider>,
    );

    expect(screen.getAllByText('hmm')).toHaveLength(1);
    expect(screen.queryByText(/Short feedback/)).toBeNull();
  });
});
