'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileText } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogTrigger } from '@/components/ui/dialog';
import { translateUI as t } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';

export interface SummaryTemplate {
  name: string;
  description: string;
  sections: Array<{ title: string; instruction: string; format: string; item_format?: string; example_item_format?: string }>;
}
type TemplateInfo = { id: string; name: string; description: string; source?: string };
const fieldClass = 'w-full rounded-md border border-input bg-background px-3 py-2 text-sm';

export function SummaryTemplateEditor({ templates, selected, onSelect, disabled }: {
  templates: TemplateInfo[];
  selected: string;
  onSelect: (id: string, name: string) => void;
  disabled?: boolean;
}) {
  useUiTranslation();
  const [open, setOpen] = useState(false);
  const [id, setId] = useState(selected);
  const [draft, setDraft] = useState<SummaryTemplate | null>(null);
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [isNew, setIsNew] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!open) return;
    let active = true;
    setDraft(null);
    setError('');
    setDirty(false);
    setIsNew(false);
    invoke<SummaryTemplate>('api_get_template', { templateId: id }).then((value) => {
      if (active) setDraft(value);
    }).catch((e) => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [open, id]);

  const update = (value: SummaryTemplate) => { setDraft(value); setDirty(true); };
  const save = async (copy: boolean) => {
    if (!draft) return;
    setBusy(true); setError('');
    const templateId = isNew || copy || !id.startsWith('custom_') ? `custom_${crypto.randomUUID()}` : id;
    try {
      await invoke('api_save_custom_template', { templateId, template: draft });
      window.dispatchEvent(new Event('summary-templates-changed'));
      onSelect(templateId, draft.name);
      setDirty(false); setOpen(false);
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  const sectionUpdate = (index: number, patch: Partial<SummaryTemplate['sections'][number]>) => {
    if (draft) update({ ...draft, sections: draft.sections.map((section, i) => i === index ? { ...section, ...patch } : section) });
  };
  const move = (index: number, offset: number) => {
    if (!draft) return;
    const sections = [...draft.sections];
    [sections[index], sections[index + offset]] = [sections[index + offset], sections[index]];
    update({ ...draft, sections });
  };
  return <Dialog open={open} onOpenChange={(value) => { if (!busy) { if (value) setId(selected); setOpen(value); } }}>
    <DialogTrigger asChild><Button variant="outline" size="sm" disabled={disabled} title={t('Select summary template')}>
      <FileText size={16} /><span className="ml-1">{t('Templates')}</span>
    </Button></DialogTrigger>
    <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
      <DialogTitle>{t('Summary templates')}</DialogTitle>
      <DialogDescription>{t('Customize sections and writing instructions. Saved templates stay on this device. Section titles are used exactly as entered; summary language controls the generated content.')}</DialogDescription>
      <fieldset disabled={busy} className="space-y-4 min-w-0">
        <label className="block text-sm">{t('Template')}
          <select className={fieldClass} value={id} disabled={dirty} onChange={(e) => setId(e.target.value)}>
            {templates.map((template) => <option key={template.id} value={template.id}>{t(template.name)}</option>)}
          </select>
        </label>
        <Button variant="outline" disabled={dirty || !draft} onClick={() => { setIsNew(true); update({ name: '', description: '', sections: [{ title: '', instruction: '', format: 'list' }] }); }}>{t('New template')}</Button>
        {dirty && <p className="text-xs text-muted-foreground">{t('Save or close this editor before switching templates.')}</p>}
        {!draft && !error && <p role="status">{t('Loading...')}</p>}
        {draft && <>
          <label className="block text-sm">{t('Template name')}<input className={fieldClass} value={draft.name} onChange={(e) => update({ ...draft, name: e.target.value })} /></label>
          <label className="block text-sm">{t('Description')}<input className={fieldClass} value={draft.description} onChange={(e) => update({ ...draft, description: e.target.value })} /></label>
          {draft.sections.map((section, index) => <div key={index} className="rounded-lg border p-3 space-y-2">
            <label className="block text-sm">{t('Section title')} {index + 1}<input className={fieldClass} value={section.title} onChange={(e) => sectionUpdate(index, { title: e.target.value })} /></label>
            <label className="block text-sm">{t('Writing instructions')}<textarea className={fieldClass} rows={3} value={section.instruction} onChange={(e) => sectionUpdate(index, { instruction: e.target.value })} /></label>
            <label className="block text-sm">{t('Preferred format')}<select className={fieldClass} value={section.format} onChange={(e) => sectionUpdate(index, { format: e.target.value })}>
              <option value="paragraph">{t('Paragraph')}</option><option value="list">{t('List')}</option><option value="string">{t('Short text')}</option>
            </select></label>
            <div className="flex gap-2">
              <Button variant="outline" size="sm" disabled={index === 0} onClick={() => move(index, -1)}>{t('Move up')}</Button>
              <Button variant="outline" size="sm" disabled={index === draft.sections.length - 1} onClick={() => move(index, 1)}>{t('Move down')}</Button>
              <Button variant="outline" size="sm" disabled={draft.sections.length === 1} onClick={() => update({ ...draft, sections: draft.sections.filter((_, i) => i !== index) })}>{t('Remove section')}</Button>
            </div>
          </div>)}
          <Button variant="outline" onClick={() => update({ ...draft, sections: [...draft.sections, { title: '', instruction: '', format: 'list' }] })}>{t('Add section')}</Button>
          <div className="flex flex-wrap gap-2 border-t pt-3">
            <Button onClick={() => void save(false)}>{busy ? t('Saving...') : !isNew && id.startsWith('custom_') ? t('Save and use') : t('Save as custom template')}</Button>
            {!isNew && id.startsWith('custom_') && <Button variant="outline" onClick={() => void save(true)}>{t('Save as copy')}</Button>}
            <Button variant="outline" disabled={dirty} onClick={() => { onSelect(id, draft.name); setOpen(false); }}>{t('Use this template')}</Button>
          </div>
        </>}
      </fieldset>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    </DialogContent>
  </Dialog>;
}
