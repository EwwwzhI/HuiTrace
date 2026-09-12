// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { LicensingProvider, useLicensing } from './LicensingContext';
import { invoke } from '@tauri-apps/api/core';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
afterEach(cleanup);

it('renders without upstream checks or a paywall, including legacy paywall requests', () => {
  function Consumer() {
    const { status, openActivateDialog } = useLicensing();
    return <button onClick={() => openActivateDialog({ paywall: true })}>{status.state}</button>;
  }
  render(<LicensingProvider><Consumer /></LicensingProvider>);
  fireEvent.click(screen.getByRole('button', { name: 'licensed' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(invoke).not.toHaveBeenCalled();
});
