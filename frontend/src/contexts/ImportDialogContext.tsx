'use client';
import { createContext, useContext, useCallback, ReactNode } from 'react';
import { useRecordingState } from './RecordingStateContext';
import { toast } from 'sonner';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';

interface ImportDialogContextType { openImportDialog: (filePath?: string | null) => void; }
const ImportDialogContext = createContext<ImportDialogContextType | null>(null);
export const useImportDialog = () => {
  const ctx = useContext(ImportDialogContext);
  if (!ctx) throw new Error('useImportDialog must be used within ImportDialogProvider');
  return ctx;
};
export function ImportDialogProvider({ children, onOpen }: { children: ReactNode; onOpen: (filePath?: string | null) => void }) {
  useUiTranslation();
  const { isRecording, isStopping, isProcessing, isSaving } = useRecordingState();
  const openImportDialog = useCallback(() => {
    if (isRecording || isStopping || isProcessing || isSaving) {
      toast.info(translateUI("Finish the current recording before importing audio."));
      return;
    }
    onOpen(null);
  }, [onOpen, isRecording, isStopping, isProcessing, isSaving]);
  return <ImportDialogContext.Provider value={{ openImportDialog }}>{children}</ImportDialogContext.Provider>;
}
