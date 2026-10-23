import { useState } from 'react';
import { confirmedPercent, updateCountdown, type Countdown } from '../shared/lib/progress';
import type { Snapshot } from '../shared/lib/types';

export function useProgressCountdown(data: Snapshot | null, connected: boolean, paused: boolean) {
  const [previous, setPrevious] = useState<Countdown | null>(null);
  const progress = data?.current_drop;
  const key = JSON.stringify([
    data?.account_id,
    data?.instance,
    progress?.campaign_id,
    progress?.drop_id,
    progress?.confirmed_minutes,
  ]);
  const next = updateCountdown(
    previous,
    key,
    progress && confirmedPercent(progress) !== null ? progress.remaining_seconds : null,
    !!(
      connected &&
      !paused &&
      data?.login.user_id &&
      !data.manual_mode.active &&
      data.mining?.state === 'watching' &&
      data.mining.channel_id != null &&
      data.mining.campaign_id === progress?.campaign_id &&
      data.mining.drop_id === progress?.drop_id &&
      progress?.countdown_running
    ),
    performance.now(),
  );
  if (next !== previous) setPrevious(next);
  return next;
}
