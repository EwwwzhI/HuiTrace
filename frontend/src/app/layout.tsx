'use client'

import './globals.css'
import './bento.css'
import { UiLanguageProvider } from '@/i18n/UiLanguageProvider'
import dynamic from 'next/dynamic'
import localFont from 'next/font/local'
import Sidebar from '@/components/Sidebar'
import { SidebarProvider } from '@/components/Sidebar/SidebarProvider'
import MainContent from '@/components/MainContent'
import AnalyticsProvider from '@/components/AnalyticsProvider'
import { toast } from 'sonner'
import "sonner/dist/styles.css"
import { useState, useEffect, useCallback } from 'react'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { TooltipProvider } from '@/components/ui/tooltip'
import { RecordingStateProvider } from '@/contexts/RecordingStateContext'
import { RecordingConsentProvider } from '@/contexts/RecordingConsentContext'
import { OllamaDownloadProvider } from '@/contexts/OllamaDownloadContext'
import { TranscriptProvider } from '@/contexts/TranscriptContext'
import { ConfigProvider } from '@/contexts/ConfigContext'
import { OnboardingProvider } from '@/contexts/OnboardingContext'
import { translateUI } from '@/i18n'
import { useUiTranslation } from '@/i18n/client'
import { DownloadProgressToastProvider } from '@/components/shared/DownloadProgressToast'
import { UpdateCheckProvider } from '@/components/UpdateCheckProvider'
import { RecordingPostProcessingProvider } from '@/contexts/RecordingPostProcessingProvider'
import { ImportDropOverlay } from '@/components/ImportAudio/ImportDropOverlay'
import { ImportDialogProvider } from '@/contexts/ImportDialogContext'
import { EncryptionStatusBanner } from '@/components/consent/EncryptionStatusBanner'
import { LicensingProvider } from '@/contexts/LicensingContext'
import { isAudioExtension, getAudioFormatsDisplayList } from '@/constants/audioFormats'
import { isTauri } from '@/lib/isTauri'
import { ThemeProvider, ThemeToaster } from '@/components/theme-provider'
import { TourProvider } from '@/components/tour'


const OnboardingFlow = dynamic(() => import('@/components/onboarding/OnboardingFlow').then(module => module.OnboardingFlow), { ssr: false })
const ImportAudioDialog = dynamic(() => import('@/components/ImportAudio/ImportAudioDialog').then(module => module.ImportAudioDialog), { ssr: false })


// Bundle the font source too: dev compilation must work without Google Fonts.
const dmSans = localFont({
  src: '../../public/fonts/dm-sans-latin.woff2',
  weight: '100 1000',
  style: 'normal',
  display: 'swap',
  variable: '--font-dm-sans',
})

// Module-level component — stable reference across RootLayout re-renders.
// Defined here (not inside RootLayout) so React never sees a new function type
// on re-render, which would cause unmount/remount and break initialization logic.
function ConditionalImportDialog({
  showImportDialog,
  handleImportDialogClose,
  importFilePath,
}: {
  showImportDialog: boolean;
  handleImportDialogClose: (open: boolean) => void;
  importFilePath: string | null;
}) {
  useUiTranslation();

  // Mount import listeners only while the dialog is open.
  if (!showImportDialog) {
    return null;
  }

  return (
    <ImportAudioDialog
      open={showImportDialog}
      onOpenChange={handleImportDialogClose}
      preselectedFile={importFilePath}
    />
  );
}

// export { metadata } from './metadata'

export default function RootLayout({
  children,
}: {
  children: React.ReactNode
}) {
  useUiTranslation();
  const [showOnboarding, setShowOnboarding] = useState(false)

  // Import audio state
  const [showDropOverlay, setShowDropOverlay] = useState(false)
  const [showImportDialog, setShowImportDialog] = useState(false)
  const [importFilePath, setImportFilePath] = useState<string | null>(null)

  useEffect(() => {
    // Outside the Tauri shell (browser dev-preview / design route) there is no
    // backend to ask and `invoke` would throw. Render the main shell directly so
    // the UI is inspectable without the desktop app; never show onboarding here.
    if (!isTauri()) {
      setShowOnboarding(false)
      return
    }

    // Check onboarding status first
    invoke<{ completed: boolean } | null>('get_onboarding_status')
      .then((status) => {
        const isComplete = status?.completed ?? false

        if (!isComplete) {
          console.log('[Layout] Onboarding not completed, showing onboarding flow')
          setShowOnboarding(true)
        } else {
          console.log('[Layout] Onboarding completed, showing main app')
        }
      })
      .catch((error) => {
        console.error('[Layout] Failed to check onboarding status:', error)
        // Default to showing onboarding if we can't check
        setShowOnboarding(true)
      })
  }, [])

  // Disable context menu in production
  useEffect(() => {
    if (process.env.NODE_ENV === 'production') {
      const handleContextMenu = (e: MouseEvent) => e.preventDefault();
      document.addEventListener('contextmenu', handleContextMenu);
      return () => document.removeEventListener('contextmenu', handleContextMenu);
    }
  }, []);
  useEffect(() => {
    // Listen for tray recording toggle request
    const unlisten = listen('request-recording-toggle', () => {
      console.log('[Layout] Received request-recording-toggle from tray');

      if (showOnboarding) {
        toast.error(translateUI("Please complete setup first"), {
          get description() { return translateUI("You need to finish onboarding before you can start recording."); }
        });
      } else {
        // If in main app, forward to useRecordingStart via window event
        console.log('[Layout] Forwarding to start-recording-from-sidebar');
        window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
      }
    });

    return () => {
      unlisten.then(fn => fn());
    };
  }, [showOnboarding]);

  // Handle file drop for audio import
  const handleFileDrop = useCallback((paths: string[]) => {
    // Find the first audio file
    const audioFile = paths.find(p => {
      const ext = p.split('.').pop()?.toLowerCase();
      return !!ext && isAudioExtension(ext);
    });

    if (audioFile) {
      console.log('[Layout] Audio file dropped:', audioFile);
      setImportFilePath(audioFile);
      setShowImportDialog(true);
    } else if (paths.length > 0) {
      toast.error(translateUI("Please drop an audio file"), {
        description: `Supported formats: ${getAudioFormatsDisplayList()}`
      });
    }
  }, []);

  // Listen for drag-drop events
  useEffect(() => {
    if (showOnboarding) return; // Don't handle drops during onboarding

    const unlisteners: UnlistenFn[] = [];
    const cleanedUpRef = { current: false };

    const setupListeners = async () => {
      // Drag enter/over - audio import is a standard feature.
      const unlistenDragEnter = await listen('tauri://drag-enter', () => {
        setShowDropOverlay(true);
      });
      if (cleanedUpRef.current) {
        unlistenDragEnter();
        return;
      }
      unlisteners.push(unlistenDragEnter);

      // Drag leave - hide overlay
      const unlistenDragLeave = await listen('tauri://drag-leave', () => {
        setShowDropOverlay(false);
      });
      if (cleanedUpRef.current) {
        unlistenDragLeave();
        unlisteners.forEach(u => u());
        return;
      }
      unlisteners.push(unlistenDragLeave);

      // Drop - process files
      const unlistenDrop = await listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
        setShowDropOverlay(false);
        handleFileDrop(event.payload.paths);
      });
      if (cleanedUpRef.current) {
        unlistenDrop();
        unlisteners.forEach(u => u());
        return;
      }
      unlisteners.push(unlistenDrop);
    };

    setupListeners();

    return () => {
      cleanedUpRef.current = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [showOnboarding, handleFileDrop]);

  // Handle import dialog close
  const handleImportDialogClose = useCallback((open: boolean) => {
    setShowImportDialog(open);
    if (!open) {
      setImportFilePath(null);
    }
  }, []);

  // Handler for ImportDialogProvider - opens import dialog from any child component
  const handleOpenImportDialog = useCallback((filePath?: string | null) => {
    setImportFilePath(filePath ?? null);
    setShowImportDialog(true);
  }, []);

  const handleOnboardingComplete = () => {
    console.log('[Layout] Onboarding completed, reloading app')
    setShowOnboarding(false)
    // Optionally reload the window to ensure all state is fresh
    window.location.reload()
  }

  return (
    <html lang="en" suppressHydrationWarning>
      <head><title>HuiTrace</title><meta name="application-name" content="HuiTrace" /></head>
      <body className={`${dmSans.variable} font-sans antialiased`}>
        <UiLanguageProvider><ThemeProvider attribute="class" defaultTheme="system" enableSystem>
        <AnalyticsProvider>
          <RecordingStateProvider>
            <RecordingConsentProvider>
            <TranscriptProvider>
              <ConfigProvider>
                <OllamaDownloadProvider>
                  <OnboardingProvider>
                    <UpdateCheckProvider>
                      <SidebarProvider>
                        <TooltipProvider>
                          <RecordingPostProcessingProvider>
                            <ImportDialogProvider onOpen={handleOpenImportDialog}>
                            {/* Compatibility context for legacy hooks; no licensing UI or requests. */}
                            <LicensingProvider>
                              {/* Download progress toast provider - listens for background downloads */}
                              <DownloadProgressToastProvider />

                              {/* Show onboarding or main app */}
                              {showOnboarding ? (
                                <OnboardingFlow onComplete={handleOnboardingComplete} />
                              ) : (
                                // First-run product tour lives ONLY in the main-app shell
                                // (never during setup onboarding). It renders the welcome
                                // overlay + coach-marks and, post-onboarding, routes to the
                                // pre-seeded sample meeting. isTauri()-gated internally.
                                <TourProvider>
                                  <div className="flex h-screen overflow-hidden bg-sidebar">
                                    <Sidebar />
                                    <MainContent>
                                      {/* ADR-0014: warns when the local DB opened UNENCRYPTED at rest.
                                          Renders nothing in the normal (encrypted) case. Sits above
                                          every main-app view and the recording indicator; not shown
                                          during onboarding (DB may not be initialized yet). */}
                                      <EncryptionStatusBanner />
                                      {children}
                                    </MainContent>
                                  </div>
                                </TourProvider>
                              )}
                              {/* Import audio overlay and dialog */}
                              <ImportDropOverlay visible={showDropOverlay} />
                              <ConditionalImportDialog
                                showImportDialog={showImportDialog}
                                handleImportDialogClose={handleImportDialogClose}
                                importFilePath={importFilePath}
                              />
                            </LicensingProvider>
                            </ImportDialogProvider>
                          </RecordingPostProcessingProvider>
                        </TooltipProvider>
                      </SidebarProvider>
                    </UpdateCheckProvider>
                  </OnboardingProvider>

                </OllamaDownloadProvider>
              </ConfigProvider>
            </TranscriptProvider>
            </RecordingConsentProvider>
          </RecordingStateProvider>
        </AnalyticsProvider>

        <ThemeToaster />
        </ThemeProvider></UiLanguageProvider>
      </body>
    </html>
  )
}
