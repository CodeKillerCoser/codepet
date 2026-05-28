import { convertFileSrc } from "@tauri-apps/api/core";
import type { AppSettings, PetEvent } from "./types";

export function shouldRing(settings: AppSettings, event: PetEvent): boolean {
  if (!event.shouldRing || settings.notifications.sound === "silent") {
    return false;
  }
  if (settings.notifications.quietHoursEnabled && isQuietNow(settings)) {
    return false;
  }
  if (event.status === "waiting-approval") {
    return settings.notifications.ringOnPermission;
  }
  if (event.status === "failed") {
    return settings.notifications.ringOnFailure;
  }
  if (event.status === "done") {
    return settings.notifications.ringOnDone;
  }
  return false;
}

export async function playNotificationSound(settings: AppSettings): Promise<void> {
  const sound = settings.notifications.sound;
  if (sound === "silent") {
    return;
  }
  if (sound === "custom" && settings.notifications.customSoundPath) {
    const audio = new Audio(convertFileSrc(settings.notifications.customSoundPath));
    await audio.play();
    return;
  }

  const audioContext = new AudioContext();
  const gain = audioContext.createGain();
  gain.connect(audioContext.destination);
  gain.gain.setValueAtTime(0.001, audioContext.currentTime);
  gain.gain.exponentialRampToValueAtTime(0.18, audioContext.currentTime + 0.02);
  gain.gain.exponentialRampToValueAtTime(0.001, audioContext.currentTime + 0.38);

  const oscillator = audioContext.createOscillator();
  oscillator.type = "square";
  oscillator.frequency.setValueAtTime(frequencyFor(sound), audioContext.currentTime);
  oscillator.frequency.setValueAtTime(frequencyFor(sound) * 1.34, audioContext.currentTime + 0.13);
  oscillator.connect(gain);
  oscillator.start();
  oscillator.stop(audioContext.currentTime + 0.42);
}

function frequencyFor(sound: AppSettings["notifications"]["sound"]): number {
  if (sound === "bell") {
    return 880;
  }
  if (sound === "chime") {
    return 660;
  }
  return 520;
}

function isQuietNow(settings: AppSettings): boolean {
  const now = new Date();
  const minutes = now.getHours() * 60 + now.getMinutes();
  const start = parseTime(settings.notifications.quietHoursStart);
  const end = parseTime(settings.notifications.quietHoursEnd);
  if (start <= end) {
    return minutes >= start && minutes < end;
  }
  return minutes >= start || minutes < end;
}

function parseTime(value: string): number {
  const [hour, minute] = value.split(":").map((part) => Number(part));
  return hour * 60 + minute;
}
