// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { RecordingConsentProvider, useRecordingConsent } from './RecordingConsentContext';
import RecordingConsentSettings from '@/components/RecordingConsentSettings';
const store = vi.hoisted(() => ({
  data: new Map<string, unknown>(),
  get: vi.fn(async (key: string): Promise<unknown> => store.data.get(key)),
  set: vi.fn(async (key: string, value: unknown) => { store.data.set(key, value); }),
  save: vi.fn(async () => {}),
}));
vi.mock('@tauri-apps/plugin-store', () => ({ load: vi.fn(async () => store) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => {}) }));
const start = vi.fn();
function Probe() {
  const { ensureRecordingConsent } = useRecordingConsent();
  return <button onClick={async () => { if (await ensureRecordingConsent()) start(); }}>Record</button>;
}
function setup() { render(<RecordingConsentProvider><Probe /><RecordingConsentSettings /></RecordingConsentProvider>); }
async function open() { fireEvent.click(screen.getByText('Record')); await screen.findByRole('dialog'); }
afterEach(cleanup);
beforeEach(() => { vi.clearAllMocks(); store.data.clear(); store.save.mockResolvedValue(); });
it('allows only this recording by default', async () => {
  setup(); await open();
  expect((screen.getByRole('checkbox') as HTMLInputElement).checked).toBe(false);
  fireEvent.click(screen.getByText('Allow & start recording'));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  expect(store.save).not.toHaveBeenCalled();
  await open();
});
it('cancel and Escape never start or save and reset the checkbox', async () => {
  setup(); await open(); fireEvent.click(screen.getByRole('checkbox')); fireEvent.click(screen.getByText('Cancel'));
  expect(start).not.toHaveBeenCalled(); expect(store.save).not.toHaveBeenCalled();
  await open(); expect((screen.getByRole('checkbox') as HTMLInputElement).checked).toBe(false);
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull()); expect(start).not.toHaveBeenCalled();
});
it('remember synchronizes settings and disabling it re-arms confirmation', async () => {
  store.data.set('recordingConsentAlwaysAsk', true); setup(); await open();
  fireEvent.click(screen.getByRole('checkbox')); fireEvent.click(screen.getByText('Allow & start recording'));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(screen.getByRole('switch').getAttribute('aria-checked')).toBe('true'));
  fireEvent.click(screen.getByText('Record')); await waitFor(() => expect(start).toHaveBeenCalledTimes(2));
  expect(screen.queryByRole('dialog')).toBeNull(); fireEvent.click(screen.getByRole('switch'));
  await waitFor(() => expect(screen.getByRole('switch').getAttribute('aria-checked')).toBe('false')); await open();
});
it('failed persistence keeps dialog open and restores memory; retry succeeds', async () => {
  store.save.mockRejectedValueOnce(new Error('disk full')); setup(); await open();
  fireEvent.click(screen.getByRole('checkbox')); fireEvent.click(screen.getByText('Allow & start recording'));
  await screen.findByRole('alert'); expect(start).not.toHaveBeenCalled();
  expect(store.data.get('recordingConsentAcknowledged')).toBe(false);
  fireEvent.click(screen.getByText('Allow & start recording')); await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
});
it('duplicate triggers do not duplicate the recording', async () => {
  setup(); fireEvent.click(screen.getByText('Record')); fireEvent.click(screen.getByText('Record'));
  await screen.findByRole('dialog'); const confirm = screen.getByText('Allow & start recording'); fireEvent.click(confirm);
  fireEvent.click(confirm); await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
});
