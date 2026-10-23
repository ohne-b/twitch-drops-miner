import { useEffect, useState } from 'react';
import { useMiner } from '../../app/MinerProvider';
import { useT } from '../lib/i18n';
import {
  confirmedPercent,
  countdownSeconds,
  formatCountdown,
  formatProgressTime,
} from '../lib/progress';
import type { Progress } from '../lib/types';

export function ProgressReadout({
  progress,
  campaignId,
  dropId,
  className = '',
}: {
  progress: Pick<Progress, 'confirmed_at' | 'confirmed_minutes' | 'required_minutes'>;
  campaignId: string;
  dropId: string;
  className?: string;
}) {
  const { data, autosave, countdown } = useMiner();
  const settings = autosave.draft ?? data?.settings;
  const t = useT();
  const [, tick] = useState(0);
  const active = data?.current_drop;
  const clock = active?.campaign_id === campaignId && active.drop_id === dropId ? countdown : null;
  const showCountdown = settings?.show_progress_countdown ?? false;
  useEffect(() => {
    if (!showCountdown || clock?.startedAt == null) return;
    const interval = window.setInterval(() => tick((value) => value + 1), 1000);
    return () => window.clearInterval(interval);
  }, [showCountdown, clock]);

  const percent = confirmedPercent(progress);
  const time =
    settings?.show_progress_time !== false
      ? formatProgressTime(
          progress.confirmed_at ? (progress.confirmed_minutes ?? 0) : 0,
          progress.required_minutes,
          settings?.progress_time_format,
          t,
        )
      : '';
  const percentage = settings?.show_progress_percentage && percent !== null ? `${percent}%` : '';
  const label = time && percentage ? `${time} (${percentage})` : time || percentage;
  if (!label && !showCountdown) return null;
  const seconds =
    percent === null
      ? null
      : clock
        ? countdownSeconds(clock, performance.now())
        : Math.max(0, progress.required_minutes - (progress.confirmed_minutes ?? 0)) * 60;
  return (
    <div
      className={`flex flex-wrap items-baseline gap-x-3 gap-y-1 text-[13px] tabular-nums ${className}`}
    >
      {label && (
        <span
          title={progress.confirmed_at ? undefined : t('progress_unknown')}
          aria-description={progress.confirmed_at ? undefined : t('progress_unknown')}
        >
          {label}
        </span>
      )}
      {showCountdown && (
        <span
          title={t('progress_countdown_help')}
          aria-label={
            seconds === null
              ? t('progress_unknown')
              : t('progress_countdown_label', { time: formatCountdown(seconds) })
          }
        >
          {seconds === null
            ? '--:--:--'
            : t('progress_countdown', { time: formatCountdown(seconds) })}
        </span>
      )}
    </div>
  );
}
