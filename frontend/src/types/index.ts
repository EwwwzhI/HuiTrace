export interface Message {
  id: string;
  content: string;
  timestamp: string;
}

export interface Transcript {
  id: string;
  text: string;
  timestamp: string; // Wall-clock time (e.g., "14:30:05")
  sequence_id?: number;
  chunk_start_time?: number; // Legacy field
  is_partial?: boolean;
  confidence?: number;
  // NEW: Recording-relative timestamps for playback sync
  audio_start_time?: number; // Seconds from recording start (e.g., 125.3)
  audio_end_time?: number;   // Seconds from recording start (e.g., 128.6)
  duration?: number;          // Segment duration in seconds (e.g., 3.3)
  asr_confidence?: number;
  speaker_id?: string;
  speaker_confidence?: number;
  speaker_provisional?: boolean;
  speaker_revision?: number;
  segment_kind?: string;
  audio_source?: 'microphone' | 'system' | 'imported' | 'mixed';
  speaker_assignment_method?: 'diarization' | 'short_turn_refinement' | 'manual';
  speaker_overlap?: boolean;
}

export interface TranscriptUpdate {
  text: string;
  timestamp: string; // Wall-clock time for reference
  source: string;
  sequence_id: number;
  chunk_start_time: number; // Legacy field
  is_partial: boolean;
  confidence: number;
  // NEW: Recording-relative timestamps for playback sync
  audio_start_time: number; // Seconds from recording start
  audio_end_time: number;   // Seconds from recording start
  duration: number;          // Segment duration in seconds
  asr_confidence?: number;
  speaker_id?: string;
  speaker_confidence?: number;
  speaker_provisional?: boolean;
  speaker_revision?: number;
  segment_kind?: string;
  audio_source?: 'microphone' | 'system' | 'imported' | 'mixed';
}

export interface Block {
  id: string;
  type: string;
  content: string;
  color: string;
}

export interface Section {
  title: string;
  blocks: Block[];
}

export interface Summary {
  [key: string]: Section;
}

export interface ApiResponse {
  message: string;
  num_chunks: number;
  data: any[];
}

export interface SummaryResponse {
  status: string;
  summary: Summary;
  raw_summary?: string;
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}

// BlockNote-specific types
//
// 'structured' (BACKLOG C1.6) is the NEW source-linked HITL draft format. It is
// detected FIRST in BlockNoteSummaryView and renders the DraftSummaryView review
// surface instead of the editable BlockNote/markdown/legacy views.
export type SummaryFormat = 'structured' | 'legacy' | 'markdown' | 'blocknote';

// Source-linked summary draft types (BACKLOG C1.6). The canonical definitions
// live in the typed service layer next to the invoke() wrappers; re-exported
// here so components can import draft types from '@/types' alongside the rest.
export type {
  BlockStatus,
  SummaryStatus,
  DraftBlockType,
  DraftBlock,
  DraftSection,
  MeetingNotesDraft,
  ActionItemDraft,
  SummaryDraftResponse,
  FieldPatch,
  EditActionItemRequest,
} from '@/services/summaryDraftService';

export interface BlockNoteBlock {
  id: string;
  type: string;
  props?: Record<string, any>;
  content?: any[];
  children?: BlockNoteBlock[];
}

export interface SummaryDataResponse {
  markdown?: string;
  summary_json?: BlockNoteBlock[];
  // Legacy format fields
  MeetingName?: string;
  _section_order?: string[];
  [key: string]: any; // For legacy section data
}

// Pagination types for optimized transcript loading
export interface MeetingMetadata {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  folder_path?: string;
}

export interface PaginatedTranscriptsResponse {
  transcripts: Transcript[];
  total_count: number;
  has_more: boolean;
}

// Transcript segment data for virtualized display
export interface TranscriptSegmentData {
  id: string;
  timestamp: number; // audio_start_time in seconds
  endTime?: number; // audio_end_time in seconds
  text: string;
  confidence?: number;
  asr_confidence?: number;
  speaker_id?: string;
  speaker_confidence?: number;
  speaker_provisional?: boolean;
  speaker_revision?: number;
  segment_kind?: string;
  audio_source?: string;
  speaker_assignment_method?: string;
  speaker_overlap?: boolean;
  /** Raw transcript ids represented by a derived utterance. Raw rows omit it. */
  source_chunk_ids?: string[];
  reconstructed?: boolean;
  embedded_events?: ReconstructedEvent[];
}

export type SpeakerAttribution =
  | { kind: 'single'; speaker_key: string }
  | { kind: 'mixed'; speaker_keys: string[] }
  | { kind: 'unknown' };

export type BoundaryReason =
  | 'same_speaker'
  | 'speaker_changed'
  | 'speaker_unknown'
  | 'short_gap'
  | 'medium_gap'
  | 'long_silence'
  | 'strong_terminal_punctuation'
  | 'weak_punctuation'
  | 'continuation_prefix'
  | 'mixed_attribution'
  | 'overlap'
  | 'overlapping_timeline'
  | 'unreliable_timing'
  | 'maximum_duration'
  | 'maximum_length'
  | 'backchannel_bridge'
  | 'score_threshold'
  | 'below_score_threshold';

export interface ReconstructedEvent {
  id: string;
  meeting_id: string;
  start_ms: number;
  end_ms: number;
  text: string;
  speaker_attribution: SpeakerAttribution;
  source_transcript_ids: string[];
  kind: 'speech' | 'backchannel' | 'noise' | 'non_speech_vocalization' | 'unknown';
  confidence?: number | null;
  overlap: boolean;
  algorithm_version: string;
}

export interface ReconstructedUtterance {
  id: string;
  meeting_id: string;
  start_ms: number;
  end_ms: number;
  speaker_attribution: SpeakerAttribution;
  text: string;
  source_transcript_ids: string[];
  reconstruction_confidence: number;
  reconstruction_reasons: BoundaryReason[];
  overlap: boolean;
  mixed: boolean;
  embedded_events: ReconstructedEvent[];
  algorithm_version: string;
}

export interface UtteranceReconstructionResult {
  meeting_id: string;
  algorithm_version: string;
  utterances: ReconstructedUtterance[];
  events: ReconstructedEvent[];
  boundaries: Array<{
    score: number;
    decision: 'split' | 'merge';
    reasons: BoundaryReason[];
  }>;
}

export type ShortTurnCandidateSource = 'transcript' | 'diarizer_turn' | 'vad_event';

export interface ShortTurnEvent {
  id: string;
  meeting_id: string;
  start_ms: number;
  end_ms: number;
  transcript_id?: string | null;
  kind: 'speech' | 'backchannel' | 'noise' | 'non_speech_vocalization' | 'unknown';
  kind_confidence: number;
  speaker_key?: string | null;
  speaker_display_name?: string | null;
  speaker_confidence?: number | null;
  automatic_speaker_key?: string | null;
  automatic_speaker_confidence?: number | null;
  candidate_sources: ShortTurnCandidateSource[];
  audio_source: 'microphone' | 'system' | 'imported' | 'mixed';
  revision: number;
  assignment_method: 'diarization' | 'short_turn_refinement' | 'manual';
  transcript_aligned: boolean;
  user_visible: boolean;
}
