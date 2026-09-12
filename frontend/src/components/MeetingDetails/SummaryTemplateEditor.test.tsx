// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { SummaryTemplateEditor } from './SummaryTemplateEditor';
import { uiI18n } from '@/i18n';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const template = { name: 'Meeting', description: 'General meeting', sections: [{ title: 'Summary', instruction: 'Summarize the discussion', format: 'paragraph' }] };
const templates = [{ id: 'standard_meeting', ...template, source: 'builtin' }];
beforeEach(async () => {
  await uiI18n.changeLanguage('en');
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command) => command === 'api_get_template' ? structuredClone(template) : undefined);
});
afterEach(cleanup);

it('opens a visible editor, saves a built-in copy and selects it for generation', async () => {
  const onSelect = vi.fn();
  const refreshed = vi.fn();
  window.addEventListener('summary-templates-changed', refreshed, { once: true });
  render(<SummaryTemplateEditor templates={templates} selected="standard_meeting" onSelect={onSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'Templates' }));
  fireEvent.change(await screen.findByLabelText('Template name'), { target: { value: '中文纪要' } });
  fireEvent.change(screen.getByLabelText('Section title 1'), { target: { value: '关键结论' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save as custom template' }));
  await waitFor(() => expect(onSelect).toHaveBeenCalledWith(expect.stringMatching(/^custom_/), '中文纪要'));
  expect(invoke).toHaveBeenCalledWith('api_save_custom_template', {
    templateId: expect.stringMatching(/^custom_/),
    template: { ...template, name: '中文纪要', sections: [{ ...template.sections[0], title: '关键结论' }] },
  });
  expect(refreshed).toHaveBeenCalledOnce();
});

it('preserves edits when saving fails and does not select the unsaved template', async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'api_save_custom_template') throw new Error('Disk unavailable');
    return structuredClone(template);
  });
  const onSelect = vi.fn();
  render(<SummaryTemplateEditor templates={templates} selected="standard_meeting" onSelect={onSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'Templates' }));
  await screen.findByLabelText('Template name');
  fireEvent.click(screen.getByRole('button', { name: 'Save as custom template' }));
  expect((await screen.findByRole('alert')).textContent).toContain('Disk unavailable');
  expect(onSelect).not.toHaveBeenCalled();
  expect((screen.getByLabelText('Template name') as HTMLInputElement).value).toBe('Meeting');
});

it('new templates never overwrite the selected custom template', async () => {
  const onSelect = vi.fn();
  render(<SummaryTemplateEditor templates={[{ ...templates[0], id: 'custom_existing', source: 'custom' }]} selected="custom_existing" onSelect={onSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'Templates' }));
  await screen.findByLabelText('Template name');
  fireEvent.click(screen.getByRole('button', { name: 'New template' }));
  fireEvent.change(screen.getByLabelText('Template name'), { target: { value: 'New meeting' } });
  fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'New template description' } });
  fireEvent.change(screen.getByLabelText('Section title 1'), { target: { value: 'Outcomes' } });
  fireEvent.change(screen.getByLabelText('Writing instructions'), { target: { value: 'Summarize outcomes' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save as custom template' }));
  await waitFor(() => expect(onSelect).toHaveBeenCalled());
  expect(onSelect.mock.calls[0][0]).not.toBe('custom_existing');
});
