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
  timing?: TranscriptTiming;
}

export type TimingSource = 'native_token_emission' | 'native_token' | 'native_word' | 'native_segment' | 'forced_alignment';

export interface TimedToken {
  text: string;
  start_ms: number;
  end_ms?: number | null;
  confidence?: number | null;
  timing_source: TimingSource;
}

export interface TranscriptTiming {
  provider: string;
  capabilities: {
    segment_timestamps: boolean;
    token_timestamps: boolean;
    word_timestamps: boolean;
    token_confidence: boolean;
  };
  tokens: TimedToken[];
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
  timing?: TranscriptTiming;
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
  speaker_attribution?: SpeakerAttribution;
}

export type SpeakerAttribution =
  | { kind: 'single'; speaker_key: string }
  | { kind: 'mixed'; speaker_keys: string[] }
  | { kind: 'unknown' };

export interface SourceLexicalRange {
  source_transcript_id: string;
  lexical_start_index: number;
  lexical_end_index: number;
  token_start_index: number;
  token_end_index: number;
}

export type BoundaryReason =
  | 'same_speaker'
  | 'speaker_changed'
  | 'reliable_speaker_change'
  | 'ambiguous_speaker_change'
  | 'speaker_unknown'
  | 'short_gap'
  | 'medium_gap'
  | 'long_silence'
  | 'strong_terminal_punctuation'
  | 'weak_punctuation'
  | 'continuation_prefix'
  | 'sentence_complete'
  | 'sentence_incomplete'
  | 'semantic_continuity'
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
  source_ranges?: SourceLexicalRange[];
}

export interface ReconstructedUtterance {
  id: string;
  meeting_id: string;
  start_ms: number;
  end_ms: number;
  speaker_attribution: SpeakerAttribution;
  text: string;
  source_transcript_ids: string[];
  mean_asr_confidence?: number | null;
  reconstruction_reasons: BoundaryReason[];
  overlap: boolean;
  mixed: boolean;
  embedded_events: ReconstructedEvent[];
  algorithm_version: string;
  source_ranges?: SourceLexicalRange[];
}

export interface ReconstructionMetrics {
  total_chunks: number;
  chunks_with_timing: number;
  valid_timing_chunks: number;
  timing_coverage: number;
  valid_timing_rate: number;
  total_lexical_units: number;
  assigned_lexical_units: number;
  word_assignment_coverage: number;
  ambiguous_count: number;
  ambiguous_rate: number;
  mixed_count: number;
  mixed_rate: number;
  cross_speaker_raw_chunk_count: number;
  resolved_cross_speaker_chunk_count: number;
  resolved_cross_speaker_chunk_rate: number;
  fallback_to_v1_count: number;
  fallback_to_v1_rate: number;
  lexical_preservation_failure_count: number;
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
    evidence: {
      left_source_transcript_id: string;
      right_source_transcript_id: string;
      gap_ms?: number | null;
      same_speaker?: boolean | null;
      speaker_change_confidence?: number | null;
      speaker_change_reliable: boolean;
      timing_reliable: boolean;
      strong_terminal_punctuation: boolean;
      weak_punctuation: boolean;
      continuation_prefix: boolean;
      projected_duration_ms: number;
      projected_text_length: number;
      mixed_attribution: boolean;
      overlap: boolean;
      backchannel_between: boolean;
      semantic: {
        baseline_version: string;
        left_completeness?: number | null;
        cross_boundary_continuity?: number | null;
      };
      prosody: { available: boolean };
    };
    score_components: {
      timing_score: number;
      speaker_score: number;
      punctuation_score: number;
      semantic_score: number;
      structural_score: number;
    };
  }>;
  config_version: string;
  config_hash: string;
  config: Record<string, number | boolean>;
  metrics: ReconstructionMetrics;
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
