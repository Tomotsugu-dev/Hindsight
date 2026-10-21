// The island counts timers down itself, so a plugin does not call island.show every second.
import type { Timer } from "./types";

/** The time left at `now` (ms), in whole seconds and never below 0. A paused timer stays at the
 * time it had left when it paused, and a finished one stays at 0 until the plugin changes it. */
export function secondsLeft(timer: Timer, now: number): number {
  const at = timer.pausedAt ? Date.parse(timer.pausedAt) : now;
  return Math.max(0, Math.ceil((Date.parse(timer.end) - at) / 1000));
}

/** The share of the timer still left at `now`, from 1 down to 0. */
export function shareLeft(timer: Timer, now: number): number {
  const total = (Date.parse(timer.end) - Date.parse(timer.start)) / 1000;
  return Math.min(1, Math.max(0, secondsLeft(timer, now) / total));
}

/** `m:ss`, or `h:mm:ss` from an hour on. */
export function formatSeconds(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const ss = String(seconds % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
