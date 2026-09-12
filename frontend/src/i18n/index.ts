import { createInstance } from 'i18next';
import en from './en.json';
import zh from './zh-CN.json';

export const uiI18n = createInstance();
void uiI18n.init({
  resources: { en: { translation: en }, 'zh-CN': { translation: zh } },
  lng: 'en', fallbackLng: 'en', supportedLngs: ['en', 'zh-CN'],
  keySeparator: false, nsSeparator: false,
  initAsync: false, interpolation: { escapeValue: false },
  react: { useSuspense: false },
});
export const translateUI = (key: string, options?: Record<string, unknown>): string =>
  String(uiI18n.t(key, options ?? {}));
export type UiLanguage = 'system' | 'zh-CN' | 'en';
export function resolveUiLanguage(choice: UiLanguage, systemLanguages: readonly string[]): 'zh-CN' | 'en' {
  if (choice !== 'system') return choice;
  return systemLanguages[0]?.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en';
}
export function validUiLanguage(value: unknown): UiLanguage {
  return value === 'zh-CN' || value === 'en' ? value : 'system';
}
