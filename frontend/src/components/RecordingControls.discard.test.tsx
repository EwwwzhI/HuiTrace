// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { RecordingControls } from './RecordingControls';
const mocks = vi.hoisted(() => ({ discard: vi.fn(), clear: vi.fn(), stop: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => false) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/analytics', () => ({ default: { trackButtonClick: vi.fn() } }));
vi.mock('@/services/recordingService', () => ({ recordingService: { discardRecording: mocks.discard } }));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => ({ discardCurrentTranscript: mocks.clear }) }));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isPaused: true }) }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => ({ setIsMeetingActive: vi.fn(), setCurrentMeeting: vi.fn() }) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
function setup() { render(<RecordingControls isRecording onRecordingStop={mocks.stop} onRecordingStart={vi.fn()} onTranscriptReceived={vi.fn()} isRecordingDisabled={false} isParentProcessing={false} />); }
it('cancel keeps recording; confirmed discard clears recovery without normal save', async () => {
  mocks.discard.mockResolvedValue(true); mocks.clear.mockResolvedValue(undefined); setup();
  fireEvent.click(screen.getByRole('button', { name: 'Discard recording' }));
  fireEvent.click(screen.getByText('Keep recording'));
  expect(mocks.discard).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Discard recording' }));
  fireEvent.click(screen.getByRole('dialog').querySelector('button.bg-destructive')!);
  await waitFor(() => expect(mocks.clear).toHaveBeenCalledOnce());
  expect(mocks.stop).not.toHaveBeenCalled();
});
it('retries recovery cleanup without discarding the backend twice', async () => {
  mocks.discard.mockResolvedValue(true); mocks.clear.mockRejectedValueOnce(new Error('disk')).mockResolvedValue(undefined); setup();
  fireEvent.click(screen.getByRole('button', { name: 'Discard recording' }));
  fireEvent.click(screen.getByRole('dialog').querySelector('button.bg-destructive')!);
  await screen.findByRole('alert'); fireEvent.click(screen.getByText('Retry cleanup'));
  await waitFor(() => expect(mocks.clear).toHaveBeenCalledTimes(2));
  expect(mocks.discard).toHaveBeenCalledOnce(); expect(mocks.stop).not.toHaveBeenCalled();
});
