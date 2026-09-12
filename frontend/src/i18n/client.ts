'use client';

import { useSyncExternalStore } from 'react';
import { translateUI, uiI18n } from './index';

const subscribeToLanguage = (onChange: () => void) => {
  uiI18n.on('languageChanged', onChange);
  return () => uiI18n.off('languageChanged', onChange);
};

/** Re-render a client component whenever the selected UI language changes. */
export function useUiTranslation() {
  useSyncExternalStore(subscribeToLanguage, () => uiI18n.language, () => 'en');
  return { t: translateUI };
}
