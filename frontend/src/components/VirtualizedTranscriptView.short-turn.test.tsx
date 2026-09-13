/** @vitest-environment jsdom */

import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { VirtualizedTranscriptView } from './VirtualizedTranscriptView';
import { TooltipProvider } from './ui/tooltip';

vi.mock('@/i18n/client', () => ({ useUiTranslation: () => undefined }));
vi.mock('framer-motion', () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
  motion: new Proxy({}, { get: (_target, tag) => tag }),
}));

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
});
