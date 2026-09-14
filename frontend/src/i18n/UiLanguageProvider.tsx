'use client';
import React, { createContext, useContext, useEffect, useState, useRef } from 'react';
import { uiI18n, resolveUiLanguage, validUiLanguage, type UiLanguage } from './index';
import { useUiTranslation } from './client';
import { isTauri } from '@/lib/isTauri';

const KEY = 'huitrace.uiLanguage';
const Context = createContext<{ choice: UiLanguage; setChoice: (choice: UiLanguage) => void }>({ choice: 'system', setChoice: () => {} });
async function nativeStore() {
  const { load } = await import('@tauri-apps/plugin-store');
  return load('ui-preferences.json', { autoSave: false, defaults: {} });
}
function applyLanguage(choice: UiLanguage) {
  const resolved = resolveUiLanguage(choice, navigator.languages);
  void uiI18n.changeLanguage(resolved);
  document.documentElement.lang = resolved;
  if (isTauri()) void import('@tauri-apps/api/core').then(({ invoke }) => invoke('set_ui_language', { language: resolved })).catch(console.error);
}
export function UiLanguageProvider({ children }: { children: React.ReactNode }) {
  const selectionVersion = useRef(0);
  const saves = useRef(Promise.resolve());
  const [choice, setChoiceState] = useState<UiLanguage>('system');
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let alive = true;
    const version = selectionVersion.current;
    let cached: UiLanguage = 'system';
    try { cached = validUiLanguage(localStorage.getItem(KEY)); } catch {}
    setChoiceState(cached);
    applyLanguage(cached);
    if (isTauri()) void nativeStore().then(async store => {
      const saved = await store.get<UiLanguage>('language');
      if (alive && version === selectionVersion.current && saved) { const value = validUiLanguage(saved); setChoiceState(value); applyLanguage(value); }
    }).catch(console.error);
    return () => { alive = false; };
  }, [revision]);
  const setChoice = (value: UiLanguage) => {
    selectionVersion.current++;
    setChoiceState(value);
    try { localStorage.setItem(KEY, value); } catch {}
    applyLanguage(value);
    if (isTauri()) saves.current = saves.current.then(async () => { const store = await nativeStore(); await store.set('language', value); await store.save(); }).catch(console.error);
  };
  useEffect(() => {
    const changed = () => { if (choice === 'system') applyLanguage(choice); };
    const stored = (event: StorageEvent) => { if (event.key === KEY) setRevision(v => v + 1); };
    window.addEventListener('languagechange', changed);
    window.addEventListener('storage', stored);
    return () => { window.removeEventListener('languagechange', changed); window.removeEventListener('storage', stored); };
  }, [choice]);
  return <Context.Provider value={{ choice, setChoice }}>{children}</Context.Provider>;
}
export function useUiLanguage() {
  return useContext(Context);
}
export function UiLanguageSetting() {
  const { choice, setChoice } = useUiLanguage();
  const { t } = useUiTranslation();
  return <div className="rounded-lg border border-border bg-card p-6">
    <label htmlFor="ui-language" className="block text-lg font-semibold">{t('Interface language')}</label>
    <p className="mt-1 text-sm text-muted-foreground">{t('Change the interface language. Transcription and summary languages stay separate.')}</p>
    <select id="ui-language" value={choice} onChange={e => setChoice(validUiLanguage(e.target.value))} className="mt-3 rounded-md border border-input bg-background px-3 py-2 text-sm">
      <option value="system">{t('Follow system')}</option><option value="zh-CN">简体中文</option><option value="en">English</option>
    </select>
  </div>;
}
