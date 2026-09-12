'use client';

import React, { useState, useEffect, useCallback, useRef } from 'react';
import { ChevronDown, ChevronRight, File, Settings, PanelLeftClose, PanelLeftOpen, Calendar, Home, Trash2, Mic, Square, Plus, Pencil, NotebookPen, SearchIcon, X, Upload, ListChecks } from 'lucide-react';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar } from './SidebarProvider';
import type { CurrentMeeting } from '@/components/Sidebar/SidebarProvider';
import { ConfirmationModal } from '../ConfirmationModel/confirmation-modal';
import Analytics from '@/lib/analytics';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { isTauri } from '@/lib/isTauri';
import { indexedDBService } from '@/services/indexedDBService';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { TOUR_ANCHORS } from '@/lib/tour';
import { APP_VERSION } from '@/lib/appVersion';

import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog"
import { VisuallyHidden } from "@/components/ui/visually-hidden"

import Logo from '../Logo';
import Info from '../Info';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '../ui/input-group';
import { SearchResultsList } from './SearchResultsList';
import {
  isEvidenceQuerySearchable,
  type TranscriptSearchResult,
} from '@/services/search';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



interface SidebarItem {
  id: string;
  title: string;
  type: 'folder' | 'file';
  children?: SidebarItem[];
}

const Sidebar: React.FC = () => {
  useUiTranslation();
  const router = useRouter();
  const pathname = usePathname();
  const isSettingsPage = pathname === '/settings';
  const searchJumpNonceRef = useRef(0);
  const {
    currentMeeting,
    setCurrentMeeting,
    sidebarItems,
    isCollapsed,
    toggleCollapse,
    handleRecordingToggle,
    searchTranscripts,
    searchResults,
    isSearching,
    searchError,
    meetings,
    setMeetings
  } = useSidebar();

  // Get recording state from RecordingStateContext (single source of truth)
  const { isRecording } = useRecordingState();
  const { openImportDialog } = useImportDialog();

  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set(['meetings']));
  const [searchQuery, setSearchQuery] = useState<string>('');

  // App version for the footer — read from the Tauri app (tauri.conf.json), so it
  // never goes stale on a release bump. Plain-browser renders fall back to the
  // synchronized package-manifest version when the Tauri API is absent.
  const [appVersion, setAppVersion] = useState(APP_VERSION);
  useEffect(() => {
    if (isTauri()) getVersion().then(setAppVersion).catch(() => {});
  }, []);
  // State for edit modal
  const [editModalState, setEditModalState] = useState<{ isOpen: boolean; meetingId: string | null; currentTitle: string }>({
    isOpen: false,
    meetingId: null,
    currentTitle: ''
  });
  const [editingTitle, setEditingTitle] = useState<string>('');

  // Ensure 'meetings' folder is always expanded
  useEffect(() => {
    if (!expandedFolders.has('meetings')) {
      const newExpanded = new Set(expandedFolders);
      newExpanded.add('meetings');
      setExpandedFolders(newExpanded);
    }
  }, [expandedFolders]);




  const [deleteModalState, setDeleteModalState] = useState<{ isOpen: boolean; itemId: string | null }>({ isOpen: false, itemId: null });
  const [isDeletePending, setIsDeletePending] = useState(false);

  // Keep search input responsive while the provider owns debounce, stale
  // response suppression and the local evidence-search command.
  const handleSearchChange = useCallback((value: string) => {
    setSearchQuery(value);
    searchTranscripts(value);

    if (value.trim()) {
      setExpandedFolders((current) => {
        if (current.has('meetings')) return current;
        const next = new Set(current);
        next.add('meetings');
        return next;
      });
    }
  }, [searchTranscripts]);

  const handleSearchResultSelect = useCallback((result: TranscriptSearchResult) => {
    setCurrentMeeting({ id: result.id, title: result.title });
    const jumpNonce = ++searchJumpNonceRef.current;
    const params = new URLSearchParams({
      id: result.id,
      segment: result.sourceChunkId,
      source: 'search',
      jump: `${Date.now()}-${jumpNonce}`,
    });
    router.push(`/meeting-details?${params.toString()}`);
  }, [router, setCurrentMeeting]);


  const handleDelete = async (itemId: string): Promise<boolean> => {
    console.log('Deleting meeting');

    try {
      // Browser recovery data is outside the native SQLite transaction. Purge
      // legacy saved copies and an exact matching recovery ID before asking the
      // native layer to remove its managed database/search/recording data.
      await indexedDBService.purgeSavedMeetings();
      await indexedDBService.deleteMeeting(itemId);
      if (sessionStorage.getItem('indexeddb_current_meeting_id') === itemId) {
        sessionStorage.removeItem('indexeddb_current_meeting_id');
      }

      const deletion = await invoke<{
        maintenance_pending?: boolean;
        recording_cleanup_status?: 'removed' | 'absent' | 'retained_untrusted' | 'retained_shared' | 'retained_ownership_mismatch';
      }>('api_delete_meeting', {
        meetingId: itemId,
      });
      console.log('Meeting deleted successfully');
      const updatedMeetings = meetings.filter((m: CurrentMeeting) => m.id !== itemId);
      setMeetings(updatedMeetings);

      // Track meeting deletion
      Analytics.trackMeetingDeleted();

      const recordingRetained = deletion.recording_cleanup_status?.startsWith('retained_') ?? false;
      if (recordingRetained) {
        const recordingReason = deletion.recording_cleanup_status === 'retained_shared'
          ? translateUI("The recording folder is still used by another meeting, so its files were kept.")
          : translateUI("HuiTrace could not safely verify the recording folder, so its files were kept.");
        const maintenanceNote = deletion.maintenance_pending
          ? ` ${translateUI("Encrypted database compaction is still pending and will retry automatically.")}`
          : '';
        toast.warning(translateUI("Meeting deleted; recording files kept"), {
          description: `${translateUI("The meeting, transcript, summary, search, and recovery data were deleted.")} ${recordingReason}${maintenanceNote}`
        });
      } else if (deletion.maintenance_pending) {
        toast.warning(translateUI("Meeting deleted"), {
          description: translateUI("The meeting data was deleted. Encrypted database compaction is still pending and will retry automatically.")
        });
      } else {
        toast.success(translateUI("Meeting deleted successfully"), {
          description: deletion.recording_cleanup_status === 'removed'
            ? translateUI("The meeting data and its HuiTrace-managed recording files were deleted.")
            : translateUI("The meeting, transcript, summary, search, and recovery data were deleted.")
        });
      }

      // If deleting the active meeting, navigate to home
      if (currentMeeting?.id === itemId) {
        setCurrentMeeting({ id: 'intro-call', get title() { return translateUI("+ New Call"); } });
        router.push('/');
      }

      return true;
    } catch (error) {
      console.error('Failed to delete meeting');
      toast.error(translateUI("Failed to delete meeting"), {
        description: translateUI(error instanceof Error ? error.message : String(error))
      });
      return false;
    }
  };

  const handleDeleteConfirm = async () => {
    const itemId = deleteModalState.itemId;
    if (!itemId || isDeletePending) return;

    setIsDeletePending(true);
    try {
      const deleted = await handleDelete(itemId);
      if (deleted) {
        setDeleteModalState({ isOpen: false, itemId: null });
      }
    } finally {
      setIsDeletePending(false);
    }
  };

  // Handle modal editing of meeting names
  const handleEditStart = (meetingId: string, currentTitle: string) => {
    setEditModalState({
      isOpen: true,
      meetingId: meetingId,
      currentTitle: currentTitle
    });
    setEditingTitle(currentTitle);
  };

  const handleEditConfirm = async () => {
    const newTitle = editingTitle.trim();
    const meetingId = editModalState.meetingId;

    if (!meetingId) return;

    // Prevent empty titles
    if (!newTitle) {
      toast.error(translateUI("Meeting title cannot be empty"));
      return;
    }

    try {
      await invoke('api_save_meeting_title', {
        meetingId: meetingId,
        title: newTitle,
      });

      // Update local state
      const updatedMeetings = meetings.map((m: CurrentMeeting) =>
        m.id === meetingId ? { ...m, title: newTitle } : m
      );
      setMeetings(updatedMeetings);

      // Update current meeting if it's the one being edited
      if (currentMeeting?.id === meetingId) {
        setCurrentMeeting({ id: meetingId, title: newTitle });
      }

      // Track the edit
      Analytics.trackButtonClick('edit_meeting_title', 'sidebar');

      toast.success(translateUI("Meeting title updated successfully"));

      // Close modal and reset state
      setEditModalState({ isOpen: false, meetingId: null, currentTitle: '' });
      setEditingTitle('');
    } catch (error) {
      console.error('Failed to update meeting title');
      toast.error(translateUI("Failed to update meeting title"), {
        description: error instanceof Error ? error.message : String(error)
      });
    }
  };

  const handleEditCancel = () => {
    setEditModalState({ isOpen: false, meetingId: null, currentTitle: '' });
    setEditingTitle('');
  };

  const toggleFolder = (folderId: string) => {
    // Normal toggle behavior for all folders
    const newExpanded = new Set(expandedFolders);
    if (newExpanded.has(folderId)) {
      newExpanded.delete(folderId);
    } else {
      newExpanded.add(folderId);
    }
    setExpandedFolders(newExpanded);
  };

  const renderCollapsedIcons = () => {
    if (!isCollapsed) return null;

    const isHomePage = pathname === '/';
    const isActionsPage = pathname === '/actions';
    const isMeetingPage = pathname?.includes('/meeting-details');

    return (
      <TooltipProvider>
        <div className="mt-2 flex flex-col items-center space-y-1.5">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={() => router.push('/')}
                type="button"
                aria-label={translateUI("Home")}
                aria-current={isHomePage ? 'page' : undefined}
                className={`grid h-9 w-9 place-items-center rounded-lg transition-colors duration-150 ${isHomePage ? 'bg-selected text-selected-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                  }`}
              >
                <Home className="w-5 h-5" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">
              <p>{translateUI("Home")}</p>
            </TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => router.push('/actions')}
                aria-label={translateUI("View Action Items")}
                aria-current={isActionsPage ? 'page' : undefined}
                className={`grid h-9 w-9 place-items-center rounded-lg transition-colors duration-150 ${isActionsPage ? 'bg-selected text-selected-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                  }`}
              >
                <ListChecks className="h-5 w-5" aria-hidden="true" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">
              <p>{translateUI("Action Items")}</p>
            </TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <button
                data-tour={TOUR_ANCHORS.recordButton}
                aria-label={translateUI(isRecording ? "Recording" : "Start recording")}
                onClick={handleRecordingToggle}
                disabled={isRecording}
                className={`grid h-9 w-9 place-items-center rounded-lg text-primary-foreground shadow-sm transition-all duration-150 ${isRecording ? 'bg-seal text-seal-foreground cursor-not-allowed' : 'bg-primary hover:-translate-y-0.5 hover:bg-primary/90 hover:shadow-md'}`}
              >
                {isRecording ? (
                  <Square className="w-5 h-5 text-primary-foreground" />
                ) : (
                  <Mic className="w-5 h-5 text-primary-foreground" />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">
              <p>{isRecording ? translateUI("Recording in progress...") : translateUI("Start Recording")}</p>
            </TooltipContent>
          </Tooltip>

          {(
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={() => openImportDialog()}
                  type="button"
                  aria-label={translateUI("Import audio")}
                  className="grid h-9 w-9 place-items-center rounded-lg text-muted-foreground transition-colors duration-150 hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <Upload className="w-5 h-5" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">
                <p>{translateUI("Import Audio")}</p>
              </TooltipContent>
            </Tooltip>
          )}

          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={() => {
                  if (isCollapsed) toggleCollapse();
                  toggleFolder('meetings');
                }}
                className={`grid h-9 w-9 place-items-center rounded-lg transition-colors duration-150 ${isMeetingPage ? 'bg-selected text-selected-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                  }`}
              >
                <NotebookPen className="w-5 h-5" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">
              <p>{translateUI("Meeting Notes")}</p>
            </TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={() => router.push('/settings')}
                type="button"
                aria-label={translateUI("Settings")}
                aria-current={isSettingsPage ? 'page' : undefined}
                className={`grid h-9 w-9 place-items-center rounded-lg transition-colors duration-150 ${isSettingsPage ? 'bg-selected text-selected-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                  }`}
              >
                <Settings className="w-5 h-5" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">
              <p>{translateUI("Settings")}</p>
            </TooltipContent>
          </Tooltip>

          <Info isCollapsed={isCollapsed} />
        </div>
      </TooltipProvider>
    );
  };

  const renderItem = (item: SidebarItem, depth = 0) => {
    const isExpanded = expandedFolders.has(item.id);
    const paddingLeft = `${depth * 12 + 12}px`;
    // currentMeeting also tracks recording/last-opened context; it does not
    // imply that its detail page is currently visible.
    const isActive = item.type === 'file' && (
      (pathname === '/meeting-details' && currentMeeting?.id === item.id) ||
      pathname === `/notes/${encodeURIComponent(item.id)}`
    );
    const isMeetingItem = item.id.includes('-') && !item.id.startsWith('intro-call');

    if (isCollapsed) return null;

    return (
      <div key={item.id}>
        <div
          aria-current={isActive ? 'page' : undefined}
          className={`group flex items-center transition-all duration-150 ${item.type === 'folder' && depth === 0
            ? 'mx-3 mt-3 h-9 rounded-lg p-2.5 text-sm font-medium'
            : `v2-meeting-row relative my-0.5 overflow-hidden rounded-lg px-2.5 py-2 text-[13px] transition-colors ${isActive ? 'bg-selected text-selected-foreground font-medium' :
              'text-foreground/72 hover:bg-muted hover:text-foreground'
            } cursor-pointer`
            }`}
          style={item.type === 'folder' && depth === 0 ? {} : { paddingLeft }}
          onClick={() => {
            if (item.type === 'folder') {
              toggleFolder(item.id);
            } else {
              setCurrentMeeting({ id: item.id, title: item.title });
              const basePath = item.id.startsWith('intro-call') ? '/' :
                item.id.includes('-') ? `/meeting-details?id=${item.id}` : `/notes/${item.id}`;
              router.push(basePath);
            }
          }}
        >
          {item.type === 'folder' ? (
            <>
              {item.id === 'meetings' ? (
                <Calendar className="w-4 h-4 mr-2" />
              ) : item.id === 'notes' ? (
                <Calendar className="w-4 h-4 mr-2" />
              ) : null}
              <span className={depth === 0 ? "" : "font-medium"}>{item.title}</span>
              <div className="ml-auto">
                {isExpanded ? (
                  <ChevronDown className="w-4 h-4 text-muted-foreground" />
                ) : (
                  <ChevronRight className="w-4 h-4 text-muted-foreground" />
                )}
              </div>
              {searchQuery && item.id === 'meetings' && isSearching && (
                <span className="ml-2 text-xs text-primary animate-pulse">{translateUI("Searching...")}</span>
              )}
            </>
          ) : (
            <div className="flex min-w-0 w-full flex-col">
              <div className="flex min-w-0 w-full items-center">
                {isMeetingItem ? (
                  <div className="mr-2 flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-lg bg-muted/80">
                    <File className="h-3.5 w-3.5 text-muted-foreground" />
                  </div>
                ) : (
                  <div className="mr-2 flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-lg bg-accent">
                    <Plus className="w-3.5 h-3.5 text-primary" />
                  </div>
                )}
                <span className={`min-w-0 flex-1 break-words ${isMeetingItem ? 'pr-16' : ''}`}>{item.title}</span>
                {isMeetingItem && (
                  <div className="v2-meeting-actions pointer-events-none absolute right-1 top-1/2 z-10 flex -translate-y-1/2 translate-x-1 items-center gap-0.5 rounded-lg border border-border/80 bg-card/95 p-0.5 opacity-0 backdrop-blur-sm transition-[opacity,transform] duration-150 group-hover:pointer-events-auto group-hover:translate-x-0 group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:translate-x-0 group-focus-within:opacity-100">
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleEditStart(item.id, item.title);
                      }}
                      className="hover:text-primary p-1 rounded-md hover:bg-accent flex-shrink-0"
                      aria-label={translateUI("Edit meeting title")}
                    >
                      <Pencil className="w-4 h-4" />
                    </button>
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        setDeleteModalState({ isOpen: true, itemId: item.id });
                      }}
                      className="hover:text-destructive p-1 rounded-md hover:bg-destructive/10 flex-shrink-0"
                      aria-label={translateUI("Delete meeting")}
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                )}
              </div>

            </div>
          )}
        </div>
        {item.type === 'folder' && isExpanded && item.children && (
          <div className="ml-1">
            {item.children.map(child => renderItem(child, depth + 1))}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="fixed top-0 left-0 h-screen z-40">
      <div
        className={`v2-sidebar ink-paper flex h-screen flex-col border-r border-border/60 bg-sidebar transition-[width] duration-200 ease-out ${isCollapsed ? 'w-16' : 'w-[232px]'
          }`}
      >
        {/* Header: brand · collapse toggle · search */}
        <div className="flex-shrink-0 px-3 pb-2 pt-3">
          {isCollapsed ? (
            <div className="flex flex-col items-center gap-2.5">
              <Logo isCollapsed />
              <button
                onClick={toggleCollapse}
                title={translateUI("Expand sidebar")}
                aria-label={translateUI("Expand sidebar")}
                className="grid h-8 w-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              >
                <PanelLeftOpen className="h-5 w-5" />
              </button>
            </div>
          ) : (
            <>
              <div className="mb-2.5 flex items-center justify-between gap-2">
                <Logo isCollapsed={false} />
                <button
                  onClick={toggleCollapse}
                  title={translateUI("Collapse sidebar")}
                  aria-label={translateUI("Collapse sidebar")}
                  className="grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                >
                  <PanelLeftClose className="h-[18px] w-[18px]" />
                </button>
              </div>

              <div className="relative">
                <InputGroup className="h-9 rounded-xl border-border/80 bg-card/70 shadow-none">
                  <InputGroupInput placeholder={translateUI("Search meeting evidence…")} value={searchQuery}
                    aria-label={translateUI("Search meeting evidence")}
                    onChange={(e) => handleSearchChange(e.target.value)}
                  />
                  <InputGroupAddon>
                    <SearchIcon />
                  </InputGroupAddon>
                  {searchQuery &&
                    <InputGroupAddon align={'inline-end'}>
                      <InputGroupButton
                        type="button"
                        aria-label={translateUI("Clear meeting search")}
                        onClick={() => handleSearchChange('')}
                      >
                        <X />
                      </InputGroupButton>
                    </InputGroupAddon>
                  }
                </InputGroup>
              </div>
            </>
          )}
        </div>

        {/* Main content - scrollable area */}
        <div className="flex-1 flex flex-col min-h-0">
          {/* Fixed navigation items */}
          <div className="flex-shrink-0">
            {!isCollapsed && (
              <>
                <button
                  type="button"
                  onClick={() => router.push('/')}
                  aria-current={pathname === '/' ? 'page' : undefined}
                   className={`mx-2.5 mt-3 flex h-9 w-[calc(100%_-_1.25rem)] items-center rounded-lg px-3 text-left text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary ${pathname === '/'
                    ? 'bg-selected text-selected-foreground'
                    : 'text-foreground/80 hover:bg-muted hover:text-foreground'
                    }`}
                >
                  <Home className="w-4 h-4 mr-2" />
                  <span>{translateUI("Home")}</span>
                </button>
                <button
                  type="button"
                  onClick={() => router.push('/actions')}
                  aria-current={pathname === '/actions' ? 'page' : undefined}
                   className={`mx-2.5 mt-1 flex h-9 w-[calc(100%_-_1.25rem)] items-center rounded-lg px-3 text-left text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary ${pathname === '/actions'
                    ? 'bg-selected text-selected-foreground'
                    : 'text-foreground/80 hover:bg-muted hover:text-foreground'
                    }`}
                >
                  <ListChecks className="mr-2 h-4 w-4" aria-hidden="true" />
                  <span>{translateUI("Action Items")}</span>
                </button>
              </>
            )}
          </div>

          {/* Content area */}
          <div className="flex-1 flex flex-col min-h-0">
            {renderCollapsedIcons()}
            {/* Meeting Notes folder header - fixed */}
            {!isCollapsed && (
              <div className="flex-shrink-0">
                {sidebarItems.filter(item => item.type === 'folder').map(item => (
                  <div key={item.id}>
                    <div
                      className="mx-3 mb-1 mt-5 flex h-7 items-center px-1 text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground"
                    >
                      <NotebookPen className="mr-2 h-3.5 w-3.5 text-muted-foreground" />
                      <span>{item.title}</span>
                      {searchQuery && item.id === 'meetings' && isSearching && (
                        <span className="ml-2 text-xs text-primary animate-pulse">{translateUI("Searching...")}</span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}

            {/* Scrollable meeting items */}
            {!isCollapsed && (
              <div className="v2-sidebar-meeting-scroll flex-1 overflow-y-auto custom-scrollbar min-h-0">
                {searchQuery.trim() ? (
                  <SearchResultsList
                    results={searchResults}
                    isSearching={isSearching}
                    isQueryTooShort={!isEvidenceQuerySearchable(searchQuery)}
                    error={searchError}
                    onSelect={handleSearchResultSelect}
                  />
                ) : (
                  sidebarItems
                    .filter(item => item.type === 'folder' && expandedFolders.has(item.id) && item.children)
                    .map(item => (
                      <div key={`${item.id}-children`} className="mx-2.5">
                        {item.children!.map(child => renderItem(child, 1))}
                      </div>
                    ))
                )}
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        {!isCollapsed && (

          <div className="flex-shrink-0 space-y-2 border-t border-border/60 p-3">
            {/* Primary CTA */}
            <button
              data-tour={TOUR_ANCHORS.recordButton}
                aria-label={translateUI(isRecording ? "Recording" : "Start recording")}
              onClick={handleRecordingToggle}
              disabled={isRecording}
              className={`flex w-full items-center justify-center gap-2 rounded-xl px-3 py-2.5 text-sm font-medium text-primary-foreground shadow-sm transition-all duration-150 ${isRecording ? 'bg-seal text-seal-foreground cursor-not-allowed' : 'bg-primary hover:-translate-y-0.5 hover:bg-primary/90 hover:shadow-md'}`}
            >
              {isRecording ? (
                <>
                  <span className="h-2 w-2 rounded-full bg-card animate-pulse" />
                  <span>{translateUI("Recording…")}</span>
                </>
              ) : (
                <>
                  <Mic className="w-4 h-4" />
                  <span>{translateUI("Start recording")}</span>
                </>
              )}
            </button>

            {/* Secondary actions — compact icon row */}
            <div className="flex items-center gap-1">
              {(
                <button
                  onClick={() => openImportDialog()}
                  title={translateUI("Import audio")}
                  aria-label={translateUI("Import audio")}
                  type="button"
                  className="flex-1 grid h-9 place-items-center rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <Upload className="w-[18px] h-[18px]" />
                </button>
              )}
              <button
                onClick={() => router.push('/settings')}
                type="button"
                title={translateUI("Settings")}
                aria-label={translateUI("Settings")}
                aria-current={isSettingsPage ? 'page' : undefined}
                className={`flex-1 grid h-9 place-items-center rounded-lg transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary ${isSettingsPage
                  ? 'bg-selected text-selected-foreground ring-1 ring-selected-border/30'
                  : 'text-muted-foreground hover:text-foreground hover:bg-muted'
                  }`}
              >
                <Settings className="w-[18px] h-[18px]" />
              </button>
              <div className="flex-1 grid place-items-center [&_button]:mb-0">
                <Info isCollapsed />
              </div>
              <span className="ml-auto pl-1 text-xs tabular-nums text-muted-foreground/70">v{appVersion}</span>
            </div>
          </div>
        )}
      </div>

      {/* Confirmation Modal for Delete */}
      <ConfirmationModal
        isOpen={deleteModalState.isOpen}
        title="Delete this meeting?"
        description="This deletes the transcript, summary, and search data. Associated recording files are removed only when HuiTrace can safely verify them; otherwise, they are kept."
        details="This action cannot be undone. Files outside HuiTrace's control—including backups, exports, snapshots, and browser storage—are not erased. Storage hardware may also retain physical traces."
        onConfirm={handleDeleteConfirm}
        onCancel={() => {
          if (!isDeletePending) {
            setDeleteModalState({ isOpen: false, itemId: null });
          }
        }}
        isBusy={isDeletePending}
      />

      {/* Edit Meeting Title Modal */}
      <Dialog open={editModalState.isOpen} onOpenChange={(open) => {
        if (!open) handleEditCancel();
      }}>
        <DialogContent className="sm:max-w-[425px]">
          <VisuallyHidden>
            <DialogTitle>{translateUI("Edit Meeting Title")}</DialogTitle>
          </VisuallyHidden>
          <div className="py-4">
            <h3 className="text-lg font-semibold mb-4">{translateUI("Edit Meeting Title")}</h3>
            <div className="space-y-4">
              <div>
                <label htmlFor="meeting-title" className="block text-sm font-medium text-foreground mb-2"> {translateUI("Meeting Title")} </label>
                <input
                  id="meeting-title"
                  type="text"
                  value={editingTitle}
                  onChange={(e) => setEditingTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      handleEditConfirm();
                    } else if (e.key === 'Escape') {
                      handleEditCancel();
                    }
                  }}
                  className="w-full px-3 py-2 border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-ring focus:border-transparent"
                  placeholder={translateUI("Enter meeting title")}
                  autoFocus
                />
              </div>
            </div>
          </div>
          <DialogFooter>
            <button
              onClick={handleEditCancel}
              className="px-4 py-2 text-sm font-medium text-foreground bg-secondary hover:bg-muted rounded-md transition-colors"
            > {translateUI("Cancel")} </button>
            <button
              onClick={handleEditConfirm}
              className="px-4 py-2 text-sm font-medium text-primary-foreground bg-primary hover:bg-primary/90 rounded-md transition-colors"
            > {translateUI("Save")} </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
};

export default Sidebar;
