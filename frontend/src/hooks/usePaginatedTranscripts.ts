import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Transcript, MeetingMetadata, PaginatedTranscriptsResponse, TranscriptSegmentData } from "@/types";
import { translateUI } from '@/i18n';


const DEFAULT_PAGE_SIZE = 100;

interface UsePaginatedTranscriptsProps {
    meetingId: string | null;
    /** Optional initial timestamp (in seconds) from URL for loading the correct page */
    initialTimestamp?: number;
}

interface UsePaginatedTranscriptsReturn {
    metadata: MeetingMetadata | null;
    segments: TranscriptSegmentData[];
    transcripts: Transcript[];
    isLoading: boolean;
    isLoadingMore: boolean;
    hasMore: boolean;
    totalCount: number;
    loadedCount: number;
    error: string | null;

    // Actions
    loadMore: () => Promise<void>;
    reset: () => void;
    refetch: () => Promise<void>;
}

/**
 * Convert Transcript array to TranscriptSegmentData for virtualized display
 */
function convertTranscriptsToSegments(transcripts: Transcript[]): TranscriptSegmentData[] {
    return transcripts.map(t => ({
        id: t.id,
        timestamp: t.audio_start_time ?? 0,
        endTime: t.audio_end_time,
        text: t.text,
        confidence: t.confidence,
        asr_confidence: t.asr_confidence,
        speaker_id: t.speaker_id,
        speaker_confidence: t.speaker_confidence,
        speaker_provisional: t.speaker_provisional,
        speaker_revision: t.speaker_revision,
        segment_kind: t.segment_kind,
        audio_source: t.audio_source,
        speaker_assignment_method: t.speaker_assignment_method,
        speaker_overlap: t.speaker_overlap,
    }));
}

export function usePaginatedTranscripts({
    meetingId,
    initialTimestamp,
}: UsePaginatedTranscriptsProps): UsePaginatedTranscriptsReturn {
    const [metadata, setMetadata] = useState<MeetingMetadata | null>(null);
    const [transcripts, setTranscripts] = useState<Transcript[]>([]);
    const [totalCount, setTotalCount] = useState(0);
    const [isLoading, setIsLoading] = useState(true);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [hasMore, setHasMore] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const offsetRef = useRef(0);
    const generationRef = useRef(0);
    const activeMeetingRef = useRef(meetingId);
    activeMeetingRef.current = meetingId;
    const isLoadingRef = useRef(false);

    // Reset state when meeting changes
    const reset = useCallback(() => {
        generationRef.current++;
        isLoadingRef.current = false;
        setMetadata(null);
        setTranscripts([]);
        setTotalCount(0);
        setIsLoading(true);
        setIsLoadingMore(false);
        setHasMore(false);
        setError(null);
        offsetRef.current = 0;
    }, []);

    // Load meeting metadata
    const loadMetadata = useCallback(async (): Promise<MeetingMetadata | null> => {
        if (!meetingId) return null;
        const generation = generationRef.current;
        const isCurrent = () => generation === generationRef.current && meetingId === activeMeetingRef.current;

        try {
            const data = await invoke<MeetingMetadata>('api_get_meeting_metadata', {
                meetingId,
            });
            if (!isCurrent()) return null;
            setMetadata(data);
            return data;
        } catch (err) {
            if (!isCurrent()) return null;
            console.error('Failed to load meeting metadata:', err);
            setError(translateUI("Failed to load meeting details"));
            return null;
        }
    }, [meetingId]);

    // Load transcripts at specific offset
    const loadTranscriptsAtOffset = useCallback(async (
        offset: number,
        append: boolean = true
    ): Promise<Transcript[]> => {
        if (!meetingId) return [];
        const generation = generationRef.current;
        const isCurrent = () => generation === generationRef.current && meetingId === activeMeetingRef.current;

        try {
            const response = await invoke<PaginatedTranscriptsResponse>(
                'api_get_meeting_transcripts',
                {
                    meetingId,
                    limit: DEFAULT_PAGE_SIZE,
                    offset,
                }
            );

            if (!isCurrent()) return [];
            const newTranscripts = response.transcripts;

            if (append) {
                setTranscripts(prev => {
                    // Deduplicate by id
                    const existingIds = new Set(prev.map(t => t.id));
                    const uniqueNew = newTranscripts.filter(t => !existingIds.has(t.id));
                    // Sort by audio_start_time
                    return [...prev, ...uniqueNew].sort((a, b) =>
                        (a.audio_start_time ?? 0) - (b.audio_start_time ?? 0)
                    );
                });
            } else {
                setTranscripts(newTranscripts);
            }

            setHasMore(response.has_more);
            setTotalCount(response.total_count);
            offsetRef.current = offset + newTranscripts.length;

            return newTranscripts;
        } catch (err) {
            if (!isCurrent()) return [];
            console.error('Failed to load transcripts:', err);
            setError(translateUI("Failed to load transcripts"));
            return [];
        }
    }, [meetingId]);

    // Load the next page. The synchronous ref guard prevents overlapping
    // requests without dropping a legitimate immediate follow-up request (the
    // jump-to-source loop may need several fast local pages in succession).
    const loadMore = useCallback(async () => {
        if (isLoadingRef.current || !hasMore || !meetingId || isLoading) return;

        isLoadingRef.current = true;
        const generation = generationRef.current;
        setIsLoadingMore(true);
        try {
            await loadTranscriptsAtOffset(offsetRef.current, true);
        } finally {
            if (generation === generationRef.current && meetingId === activeMeetingRef.current) {
                setIsLoadingMore(false);
                isLoadingRef.current = false;
            }
        }
    }, [hasMore, meetingId, loadTranscriptsAtOffset, isLoading]);

    // Force refetch of data (e.g., after retranscription)
    const refetch = useCallback(async () => {
        if (!meetingId) return;

        reset();
        const generation = generationRef.current;
        setIsLoading(true);
        try {
            await Promise.all([loadMetadata(), loadTranscriptsAtOffset(0, false)]);
        } finally {
            if (generation === generationRef.current && meetingId === activeMeetingRef.current) setIsLoading(false);
        }
    }, [meetingId, reset, loadMetadata, loadTranscriptsAtOffset]);

    // Initial load
    useEffect(() => {
        if (!meetingId) {
            reset();
            return;
        }

        reset();
        const generation = generationRef.current;

        const loadInitial = async () => {
            setIsLoading(true);
            try {
                await Promise.all([loadMetadata(), loadTranscriptsAtOffset(0, false)]);
            } finally {
                if (generation === generationRef.current && meetingId === activeMeetingRef.current) setIsLoading(false);
            }
        };

        loadInitial();
        return () => { generationRef.current++; };
    }, [meetingId, reset, loadMetadata, loadTranscriptsAtOffset]);

    // Convert to segments (memoized)
    const segments = useMemo(() =>
        convertTranscriptsToSegments(transcripts),
        [transcripts]
    );

    return {
        metadata,
        segments,
        transcripts,
        isLoading,
        isLoadingMore,
        hasMore,
        totalCount,
        loadedCount: transcripts.length,
        error,
        loadMore,
        reset,
        refetch,
    };
}
