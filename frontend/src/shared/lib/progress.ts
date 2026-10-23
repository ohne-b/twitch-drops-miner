import type { translator } from './i18n';
import type { Progress, Settings } from './types';

export function formatProgressTime(
  current: number,
  total: number,
  format: Settings['progress_time_format'] | undefined,
  t: ReturnType<typeof translator>,
): string {
  if (format !== 'hours_minutes') return t('minutes_progress', { current, total });
  function duration(minutes: number) {
    if (minutes < 60) return t('duration_minutes', { minutes });
    const hours = t('duration_hours', { hours: Math.floor(minutes / 60) });
    const remainder = minutes % 60;
    return remainder ? `${hours} ${t('duration_minutes', { minutes: remainder })}` : hours;
  }
  return t('time_progress', { current: duration(current), total: duration(total) });
}

export function confirmedPercent(
  progress: Pick<Progress, 'confirmed_at' | 'confirmed_minutes' | 'required_minutes'> | null,
): number | null {
  const minutes = progress?.confirmed_minutes;
  if (
    !progress?.confirmed_at ||
    typeof minutes !== 'number' ||
    !Number.isFinite(minutes) ||
    !Number.isFinite(progress.required_minutes) ||
    progress.required_minutes <= 0
  )
    return null;
  return Math.floor(Math.min(1000, Math.max(0, (minutes * 1000) / progress.required_minutes))) / 10;
}

export interface Countdown {
  key: string;
  sourceSeconds: number;
  seconds: number;
  startedAt: number | null;
}

export function countdownSeconds(countdown: Countdown, now: number): number {
  const elapsed =
    countdown.startedAt === null ? 0 : Math.max(0, (now - countdown.startedAt) / 1000);
  // Interpolate only within one minute; further progress must come from the miner.
  return Math.ceil(Math.max(0, countdown.sourceSeconds - 59, countdown.seconds - elapsed));
}

export function updateCountdown(
  previous: Countdown | null,
  key: string,
  seconds: number | null,
  running: boolean,
  now: number,
): Countdown | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return null;
  if (!previous || previous.key !== key || previous.sourceSeconds !== seconds)
    return { key, sourceSeconds: seconds, seconds, startedAt: running ? now : null };
  if (running === (previous.startedAt !== null)) return previous;
  return {
    ...previous,
    seconds: countdownSeconds(previous, now),
    startedAt: running ? now : null,
  };
}

export function formatCountdown(seconds: number): string {
  const whole = Math.max(0, Math.ceil(seconds));
  return [Math.floor(whole / 3600), Math.floor((whole % 3600) / 60), whole % 60]
    .map((part) => String(part).padStart(2, '0'))
    .join(':');
}
