export type UtteranceAnnotationMode = 'blind' | 'review';

export function canShowReconstructionEvidence(
  mode: UtteranceAnnotationMode,
  blindComplete: boolean,
) {
  return mode === 'review' && blindComplete;
}
