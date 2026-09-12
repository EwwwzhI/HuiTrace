// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { HomeOverview } from './HomeOverview';

vi.mock('./ThemeToggle', () => ({ ThemeToggle: () => null }));
afterEach(cleanup);

it('filters tools without losing their existing actions or meeting navigation', () => {
  const onImport = vi.fn(), onActions = vi.fn(), onSettings = vi.fn(), onRecord = vi.fn(), onOpen = vi.fn();
  render(<HomeOverview meetings={[{ id: 'meeting-1', title: 'Design review', date: null }]} total={1} actions={[]}
    onOpen={onOpen} onImport={onImport} onActions={onActions} onSettings={onSettings} onRecord={onRecord} />);
  fireEvent.click(screen.getByRole('button', { name: /^Capture$/ }));
  expect(screen.queryByRole('button', { name: /^Settings Your/ })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /Import audio/ }));
  expect(onImport).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: /^Organize$/ }));
  expect(screen.queryByRole('button', { name: /Import audio/ })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /SettingsYour/ }));
  fireEvent.click(screen.getByRole('button', { name: /Action ItemsReview approved/ }));
  expect(onSettings).toHaveBeenCalledOnce();
  expect(onActions).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: /^Open meeting$/ }));
  expect(onOpen).toHaveBeenCalledWith('meeting-1', 'Design review');
  fireEvent.click(screen.getByRole('button', { name: /^Start recording$/ }));
  expect(onRecord).toHaveBeenCalledOnce();
});

it('keeps recording unavailable while a start or existing operation is pending', () => {
  const onRecord = vi.fn();
  render(<HomeOverview meetings={[]} total={0} actions={[]} onOpen={vi.fn()} onRecord={onRecord} recordingDisabled />);
  fireEvent.click(screen.getByRole('button', { name: /^Start recording$/ }));
  expect(onRecord).not.toHaveBeenCalled();
});
