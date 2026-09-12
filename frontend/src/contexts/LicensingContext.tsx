'use client';

import React, { createContext, useContext } from 'react';
import type { LicensingStatus } from '@/types/licensing';

export interface OpenActivateDialogOptions {
  paywall?: boolean;
}

// Compatibility for legacy consumers: no trial, polling, or activation dialog.
const status: LicensingStatus = {
  state: 'licensed',
  daysLeft: null,
  plan: null,
  expiresAt: null,
  displayKey: null,
  reason: null,
  configured: false,
};
const value = {
  status,
  refresh: async (): Promise<void> => {},
  activate: async (_key: string): Promise<LicensingStatus> => {
    throw new Error('HuiTrace does not require a license key.');
  },
  deactivate: async (): Promise<LicensingStatus> => status,
  openActivateDialog: (_options?: OpenActivateDialogOptions): void => {},
};
const LicensingContext = createContext<typeof value | null>(null);

export function useLicensing() {
  const context = useContext(LicensingContext);
  if (!context) throw new Error('useLicensing must be used within a LicensingProvider');
  return context;
}

export function LicensingProvider({ children }: { children: React.ReactNode }) {
  return <LicensingContext.Provider value={value}>{children}</LicensingContext.Provider>;
}
