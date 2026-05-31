import { convertFileSrc } from "@tauri-apps/api/core";
import { activityKey } from "./activity";
import type { AppSettings, PetEvent, WhipReactionSound } from "./types";

const whipReactionDelay = 180;

export function shouldRing(settings: AppSettings, event: PetEvent): boolean {
  if (!event.shouldRing || settings.notifications.sound === "silent") {
    return false;
  }
  if (settings.notifications.quietHoursEnabled && isQuietNow(settings)) {
    return false;
  }
  if (event.kind === "permission-requested" && event.status === "waiting-approval") {
    return settings.notifications.ringOnPermission;
  }
  if (event.kind === "task-failed" && event.status === "failed") {
    return settings.notifications.ringOnFailure;
  }
  if (event.kind === "task-completed" && event.status === "done") {
    return settings.notifications.ringOnDone;
  }
  return false;
}

export function shouldRepeatNotification(
  settings: AppSettings,
  repeatEvent: PetEvent | null,
  currentActivities: PetEvent[],
  nowMs: number,
  repeatExpiresAt: number,
): boolean {
  if (!repeatEvent || nowMs >= repeatExpiresAt || !shouldRing(settings, repeatEvent)) {
    return false;
  }
  const repeatKey = activityKey(repeatEvent);
  return currentActivities.some((activity) => activityKey(activity) === repeatKey && activity.status === "waiting-approval");
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

export async function playWhipSound(settings?: AppSettings | null): Promise<void> {
  const audioContext = new AudioContext();
  const startTime = audioContext.currentTime;
  const gain = audioContext.createGain();
  const filter = audioContext.createBiquadFilter();
  const sampleCount = Math.floor(audioContext.sampleRate * 0.16);
  const buffer = audioContext.createBuffer(1, sampleCount, audioContext.sampleRate);
  const channel = buffer.getChannelData(0);

  for (let index = 0; index < sampleCount; index += 1) {
    const tail = 1 - index / sampleCount;
    channel[index] = (Math.random() * 2 - 1) * tail * tail;
  }

  filter.type = "highpass";
  filter.frequency.setValueAtTime(1800, startTime);
  gain.gain.setValueAtTime(0.001, startTime);
  gain.gain.exponentialRampToValueAtTime(0.32, startTime + 0.012);
  gain.gain.exponentialRampToValueAtTime(0.001, startTime + 0.16);
  filter.connect(gain);
  gain.connect(audioContext.destination);

  const crack = audioContext.createBufferSource();
  crack.buffer = buffer;
  crack.connect(filter);
  crack.start(startTime);
  crack.stop(startTime + 0.17);

  const reactionSound = settings?.pet.whipReactionSound ?? "none";
  const customSoundPath = settings?.pet.customWhipReactionSoundPath ?? null;
  const delayMs = whipReactionDelayMs(reactionSound, customSoundPath);
  if (delayMs > 0) {
    scheduleWhipReactionSound(audioContext, startTime + delayMs / 1000, reactionSound, customSoundPath);
  }
}

export function shouldPlayWhipReaction(
  sound: WhipReactionSound | null | undefined,
  customSoundPath?: string | null,
): sound is Exclude<WhipReactionSound, "none"> {
  if (sound === "custom") {
    return Boolean(customSoundPath);
  }
  return sound === "pa" || sound === "scream";
}

export function whipReactionDelayMs(sound: WhipReactionSound | null | undefined, customSoundPath?: string | null): number {
  return shouldPlayWhipReaction(sound, customSoundPath) ? whipReactionDelay : 0;
}

export async function playWhipReactionSound(
  sound: WhipReactionSound | null | undefined,
  customSoundPath?: string | null,
): Promise<void> {
  if (!shouldPlayWhipReaction(sound, customSoundPath)) {
    return;
  }

  if (sound === "custom" && customSoundPath) {
    const audio = new Audio(convertFileSrc(customSoundPath));
    await audio.play();
    return;
  }

  const audioContext = new AudioContext();
  const startTime = audioContext.currentTime;
  scheduleBuiltInWhipReaction(audioContext, startTime, sound);
}

function scheduleWhipReactionSound(
  audioContext: AudioContext,
  startTime: number,
  sound: WhipReactionSound | null | undefined,
  customSoundPath?: string | null,
) {
  if (sound === "custom" && customSoundPath) {
    const audio = new Audio(convertFileSrc(customSoundPath));
    audio.preload = "auto";
    window.setTimeout(() => {
      void audio.play();
    }, Math.max(0, Math.round((startTime - audioContext.currentTime) * 1000)));
    return;
  }

  scheduleBuiltInWhipReaction(audioContext, startTime, sound);
}

function scheduleBuiltInWhipReaction(
  audioContext: AudioContext,
  startTime: number,
  sound: Exclude<WhipReactionSound, "none" | "custom"> | "custom",
) {
  if (sound === "pa") {
    playPetSlapSound(audioContext, startTime);
    return;
  }
  if (sound === "scream") {
    playPetScreamSound(audioContext, startTime);
  }
}

function playPetSlapSound(audioContext: AudioContext, startTime: number) {
  const gain = audioContext.createGain();
  const filter = audioContext.createBiquadFilter();
  const sampleCount = Math.floor(audioContext.sampleRate * 0.12);
  const buffer = audioContext.createBuffer(1, sampleCount, audioContext.sampleRate);
  const channel = buffer.getChannelData(0);

  for (let index = 0; index < sampleCount; index += 1) {
    const progress = index / sampleCount;
    const envelope = Math.exp(-progress * 18);
    channel[index] = (Math.random() * 2 - 1) * envelope;
  }

  filter.type = "bandpass";
  filter.frequency.setValueAtTime(1200, startTime);
  filter.Q.setValueAtTime(0.9, startTime);
  gain.gain.setValueAtTime(0.001, startTime);
  gain.gain.exponentialRampToValueAtTime(0.24, startTime + 0.008);
  gain.gain.exponentialRampToValueAtTime(0.001, startTime + 0.12);
  filter.connect(gain);
  gain.connect(audioContext.destination);

  const slap = audioContext.createBufferSource();
  slap.buffer = buffer;
  slap.connect(filter);
  slap.start(startTime);
  slap.stop(startTime + 0.13);
}

function playPetScreamSound(audioContext: AudioContext, startTime: number) {
  const output = audioContext.createGain();
  output.connect(audioContext.destination);
  output.gain.setValueAtTime(0.001, startTime);
  output.gain.exponentialRampToValueAtTime(0.36, startTime + 0.02);
  output.gain.exponentialRampToValueAtTime(0.001, startTime + 0.58);

  for (let index = 0; index < 3; index += 1) {
    const start = startTime + index * 0.16;
    const duration = 0.14;
    const oscillator = audioContext.createOscillator();
    const vibrato = audioContext.createOscillator();
    const vibratoGain = audioContext.createGain();
    const syllableGain = audioContext.createGain();

    oscillator.type = "sawtooth";
    oscillator.frequency.setValueAtTime(420 + index * 34, start);
    oscillator.frequency.exponentialRampToValueAtTime(330 + index * 28, start + duration);
    vibrato.type = "sine";
    vibrato.frequency.setValueAtTime(14, start);
    vibratoGain.gain.setValueAtTime(18, start);
    vibrato.connect(vibratoGain);
    vibratoGain.connect(oscillator.frequency);
    syllableGain.gain.setValueAtTime(0.001, start);
    syllableGain.gain.exponentialRampToValueAtTime(0.28, start + 0.025);
    syllableGain.gain.exponentialRampToValueAtTime(0.001, start + duration);
    oscillator.connect(syllableGain);
    syllableGain.connect(output);
    vibrato.start(start);
    vibrato.stop(start + duration);
    oscillator.start(start);
    oscillator.stop(start + duration);
  }
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
