import { memo } from 'react';
import styles from './RecordingWaveform.module.css';

interface RecordingWaveformProps {
  isRecording: boolean;
  isPaused: boolean;
}

// Decorative recording activity, not a microphone input-level meter.
export const RecordingWaveform = memo(function RecordingWaveform({
  isRecording,
  isPaused,
}: RecordingWaveformProps) {
  const state = !isRecording ? 'idle' : isPaused ? 'paused' : 'active';

  return (
    <div className={styles.waveform} data-state={state} aria-hidden="true">
      <span className={styles.bar} />
      <span className={styles.bar} />
      <span className={styles.bar} />
    </div>
  );
});
