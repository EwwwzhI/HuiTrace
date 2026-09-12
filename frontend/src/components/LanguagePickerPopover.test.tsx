// @vitest-environment jsdom
import { useState } from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { LanguagePickerPopover } from './LanguagePickerPopover';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { uiI18n } from '@/i18n';

vi.mock('@/hooks/useRecentLanguages', () => ({ useRecentLanguages: () => ({ recents: [] }) }));
beforeEach(async () => { await uiI18n.changeLanguage('en'); });
afterEach(cleanup);

function Picker({ mode = 'meeting' }: { mode?: 'meeting' | 'settings' }) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState<string | null>(null);
  return <><button>Outside</button><Popover open={open} onOpenChange={setOpen}>
    <PopoverTrigger>Language</PopoverTrigger>
    <PopoverContent>
      <LanguagePickerPopover mode={mode} value={value} onChange={(code) => { setValue(code); setOpen(false); }} />
    </PopoverContent>
  </Popover><output>{value}</output></>;
}

// Include mousedown: click() alone would miss the original close/reopen race.
function click(element: HTMLElement) {
  fireEvent.pointerDown(element, { pointerType: 'mouse', button: 0 });
  fireEvent.mouseDown(element);
  fireEvent.mouseUp(element);
  fireEvent.click(element);
}

it.each(['meeting', 'settings'] as const)('toggles closed on the second trigger click in %s mode', (mode) => {
  render(<Picker mode={mode} />);
  const trigger = screen.getByRole('button', { name: 'Language' });
  click(trigger);
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  click(trigger);
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
  expect(screen.queryByPlaceholderText('Search language...')).toBeNull();
  click(trigger);
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
});

it('keeps search clicks open, selects a language, and closes on Escape', async () => {
  render(<Picker />);
  const trigger = screen.getByRole('button', { name: 'Language' });
  click(trigger);
  const search = screen.getByPlaceholderText('Search language...');
  click(search);
  fireEvent.change(search, { target: { value: 'Chinese' } });
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  click(screen.getByRole('button', { name: 'Chinese (zh)' }));
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
  expect(screen.getByRole('status').textContent).toBe('zh');
  click(trigger);
  fireEvent.keyDown(screen.getByPlaceholderText('Search language...'), { key: 'Escape' });
  await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('false'));
});

it('closes when clicking outside', async () => {
  render(<Picker />);
  const trigger = screen.getByRole('button', { name: 'Language' });
  click(trigger);
  // Radix installs its document pointer listener after the opening event.
  await new Promise((resolve) => setTimeout(resolve, 0));
  click(screen.getByRole('button', { name: 'Outside' }));
  await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('false'));
});
