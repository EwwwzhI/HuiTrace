// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@/hooks/useDiarization', () => ({
  useDiarization: () => ({
    state: null,
    error: null,
    turns: [],
    events: [],
    busy: false,
    run: vi.fn(),
    downloadModels: vi.fn(),
    renameSpeaker: vi.fn(),
    refresh: vi.fn(),
    assignTranscriptSpeaker: vi.fn(),
    assignShortTurnEventSpeaker: vi.fn(),
  }),
}));
vi.mock('@/components/VirtualizedTranscriptView', () => ({
  VirtualizedTranscriptView: ({ segments }: { segments: Array<{ id: string; text: string; source_chunk_ids?: string[] }> }) => (
    <div data-testid="segments">
      {segments.map((segment) => (
        <p key={segment.id} data-sources={segment.source_chunk_ids?.join(',')}>{segment.text}</p>
      ))}
    </div>
  ),
}));
vi.mock('./TranscriptButtonGroup', () => ({ TranscriptButtonGroup: () => null }));
vi.mock('./EvaluationToolsMenu', () => ({ EvaluationToolsMenu: () => null }));

const { TranscriptPanel } = await import('./TranscriptPanel');

afterEach(() => {
  cleanup();
  invoke.mockReset();
});
describe('utterance reconstruction view', () => {
  it('defaults to reconstructed utterances and retains raw source provenance', async () => {
    invoke.mockResolvedValue({
      meeting_id: 'm1',
      algorithm_version: 'utterance-reconstruction-v1',
      utterances: [{
        id: 'utterance-1',
        meeting_id: 'm1',
        start_ms: 0,
        end_ms: 2000,
        speaker_attribution: { kind: 'single', speaker_key: 'a' },
        text: '合并后的语句',
        source_transcript_ids: ['raw-1', 'raw-2'],
        reconstruction_confidence: 0.9,
        reconstruction_reasons: ['same_speaker'],
        overlap: false,
        mixed: false,
        embedded_events: [],
        algorithm_version: 'utterance-reconstruction-v1',
      }],
      events: [],
      boundaries: [],
    });

    render(
      <TranscriptPanel
        transcripts={[
          { id: 'raw-1', text: '原始一', timestamp: '00:00:00', audio_start_time: 0, audio_end_time: 1 },
          { id: 'raw-2', text: '原始二', timestamp: '00:00:01', audio_start_time: 1.2, audio_end_time: 2 },
        ]}
        customPrompt=""
        onPromptChange={() => {}}
        onCopyTranscript={() => {}}
        onOpenMeetingFolder={async () => {}}
        isRecording={false}
        meetingId="m1"
      />
    );

    await waitFor(() => expect(screen.getByText('合并后的语句')).toBeTruthy());
    expect(screen.getByText('合并后的语句').getAttribute('data-sources')).toBe('raw-1,raw-2');

    fireEvent.click(screen.getByRole('button', { name: 'Raw' }));
    expect(screen.getByText('原始一')).toBeTruthy();
    expect(screen.getByText('原始二')).toBeTruthy();
  });

  it('falls back to raw rows when reconstruction is unavailable', async () => {
    invoke.mockRejectedValue(new Error('not available'));
    render(
      <TranscriptPanel
        transcripts={[{ id: 'raw-1', text: '原始证据', timestamp: '00:00:00' }]}
        customPrompt=""
        onPromptChange={() => {}}
        onCopyTranscript={() => {}}
        onOpenMeetingFolder={async () => {}}
        isRecording={false}
        meetingId="m1"
      />
    );
    await waitFor(() => expect(screen.getByText('原始证据')).toBeTruthy());
  });
});
