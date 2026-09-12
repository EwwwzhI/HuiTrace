import { useCallback } from 'react';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { translateUI } from '@/i18n';


interface UseMeetingOperationsProps {
  meeting: any;
}

export function useMeetingOperations({
  meeting,
}: UseMeetingOperationsProps) {

  // Open meeting folder in file explorer
  const handleOpenMeetingFolder = useCallback(async () => {
    try {
      const audioPath = await invokeTauri<string | null>('api_get_meeting_audio_path', {
        meetingId: meeting.id,
      });
      if (!audioPath) {
        toast.error(translateUI("Audio is unavailable for this meeting"), {
          description: translateUI("Reimport the original audio file to restore playback."),
        });
        return;
      }
      await invokeTauri('open_meeting_folder', { meetingId: meeting.id });
    } catch (error) {
      console.error('Failed to open meeting folder:', error);
      toast.error(error as string || translateUI("Failed to open audio folder"));
    }
  }, [meeting.id]);

  return {
    handleOpenMeetingFolder,
  };
}
