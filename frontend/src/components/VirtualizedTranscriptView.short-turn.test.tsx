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
  it('resolves a raw source id to its reconstructed utterance', () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
      configurable: true,
      value: scrollIntoView,
    });
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={[{
            id: 'utterance-1', timestamp: 0, endTime: 2, text: 'Reconstructed', reconstructed: true,
            source_chunk_ids: ['raw-1', 'raw-2'], speaker_attribution: { kind: 'unknown' },
          }]}
          scrollToSegmentId="raw-2"
          disableAutoScroll enableStreaming={false} showConfidence={false}
        />
      </TooltipProvider>,
    );
    expect(scrollIntoView).toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it('does not re-infer an explicitly unknown reconstructed speaker from turns', () => {
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={[{
            id: 'unknown', timestamp: 0, endTime: 2, text: 'Unknown evidence', reconstructed: true,
            speaker_attribution: { kind: 'unknown' },
          }]}
          speakerTurns={[{ start_ms: 0, end_ms: 2000, speaker_label: 'Speaker 1', speaker_key: 'a', confidence: 0.9 }]}
          disableAutoScroll enableStreaming={false} showConfidence={false}
        />
      </TooltipProvider>,
    );
    expect(screen.getByText('Unknown speaker')).toBeTruthy();
    expect(screen.queryByText('Speaker 1')).toBeNull();
  });

  it('keeps an explicitly mixed reconstructed row mixed', () => {
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={[{
            id: 'mixed', timestamp: 0, endTime: 2, text: 'Mixed evidence', reconstructed: true,
            speaker_attribution: { kind: 'mixed', speaker_keys: ['a', 'b'] },
          }]}
          speakerTurns={[{ start_ms: 0, end_ms: 2000, speaker_label: 'Speaker 1', speaker_key: 'a', confidence: 0.9 }]}
          disableAutoScroll enableStreaming={false} showConfidence={false}
        />
      </TooltipProvider>,
    );
    expect(screen.getByText('Mixed speakers')).toBeTruthy();
    expect(screen.queryByText('Speaker 1')).toBeNull();
  });

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
