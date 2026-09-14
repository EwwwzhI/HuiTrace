// @vitest-environment jsdom
import React from 'react';
import { cleanup, render } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { WaveformTimeline } from './WaveformTimeline';

const mocks = vi.hoisted(() => {
  const plugin = {
    enableDragSelection: vi.fn(),
    on: vi.fn(),
    clearRegions: vi.fn(),
    addRegion: vi.fn(),
  };
  const wave = {
    destroy: vi.fn(), zoom: vi.fn(), getCurrentTime: vi.fn(() => 0), setTime: vi.fn(),
    setOptions: vi.fn(), on: vi.fn(),
  };
  return { plugin, wave, create: vi.fn(() => wave), theme: 'light' as 'light' | 'dark' };
});

vi.mock('next-themes', () => ({ useTheme: () => ({ resolvedTheme: mocks.theme }) }));
vi.mock('wavesurfer.js', () => ({ default: { create: mocks.create } }));
vi.mock('wavesurfer.js/dist/plugins/regions.esm.js', () => ({ default: { create: () => mocks.plugin } }));
vi.mock('wavesurfer.js/dist/plugins/minimap.esm.js', () => ({ default: { create: () => ({}) } }));
vi.mock('wavesurfer.js/dist/plugins/timeline.esm.js', () => ({ default: { create: () => ({}) } }));

afterEach(() => {
  cleanup();
  mocks.theme = 'light';
  for (const mock of [mocks.plugin.enableDragSelection, mocks.plugin.on, mocks.plugin.clearRegions, mocks.plugin.addRegion, mocks.wave.destroy, mocks.wave.zoom, mocks.wave.setTime, mocks.wave.setOptions, mocks.wave.on, mocks.create]) mock.mockClear();
});

it('updates WaveSurfer colors without recreating events or the WaveSurfer instance', () => {
  const onCreate = vi.fn();
  const media = document.createElement('audio');
  const props = {
    media,
    sourceUrl: 'meeting.wav',
    events: [{ event_id: 'meeting-demo-001-event-0001', start_ms: 100, end_ms: 500, kind: 'short_speech' }],
    selectedId: 'meeting-demo-001-event-0001',
    currentMs: 0,
    onSeek: vi.fn(),
    onCreate,
    onSelect: vi.fn(),
    onBoundsChange: vi.fn(),
  };
  const { rerender } = render(<WaveformTimeline {...props} />);
  const initialIds = mocks.plugin.addRegion.mock.calls.map(([region]) => region.id);

  mocks.theme = 'dark';
  rerender(<WaveformTimeline {...props} />);

  expect(mocks.create).toHaveBeenCalledTimes(1);
  expect(mocks.wave.setOptions).toHaveBeenLastCalledWith({ waveColor: '#67e8f9', progressColor: '#99f6e4', cursorColor: '#fcd34d' });
  expect(mocks.plugin.addRegion.mock.calls.at(-1)?.[0].id).toBe(initialIds[0]);
  expect(onCreate).not.toHaveBeenCalled();
});
