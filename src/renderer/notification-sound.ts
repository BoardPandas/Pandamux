import { useStore } from './store';
import notificationSoundUrl from './assets/notification.wav';

// One reusable <audio> element. Re-seeking to 0 lets rapid back-to-back
// notifications retrigger the chime without allocating a new decoder each time.
let audioEl: HTMLAudioElement | null = null;

/**
 * Play the default notification chime, unless the user set
 * `notificationPrefs.sound` to 'none'. Fixes issue #32: the Sound setting was
 * defined and shown in the UI but never read or wired to any audio playback.
 */
export function playNotificationSound(): void {
  if (useStore.getState().notificationPrefs.sound === 'none') return;
  try {
    if (!audioEl) {
      audioEl = new Audio(notificationSoundUrl);
      audioEl.volume = 0.45;
    }
    audioEl.currentTime = 0;
    audioEl.play().catch(() => {
      // Autoplay policy can reject before the first user gesture; harmless.
    });
  } catch {
    // Decode/playback failure is non-fatal — never let a chime break notifications.
  }
}

/**
 * Subscribe to the main process' `notification:play-sound` signal, fired once
 * for every notification (toast/flash/ring) so the chime stays in sync with
 * them. Called once at renderer startup.
 */
export function initNotificationSound(): void {
  window.wmux?.notification?.onPlaySound?.(playNotificationSound);
}
