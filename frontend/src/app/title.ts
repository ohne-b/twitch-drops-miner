import type { Snapshot } from '../shared/lib/types';

export function miningTitle(
  data: Snapshot | null,
  connected: boolean,
  t: (key: string) => string,
): string {
  const app = 'Drops Miner';
  if (!data) return app;
  if (!connected) return `${t('tab_disconnected')} - ${app}`;
  if (!data.login.user_id) return app;
  if (data.settings.mining_paused || data.mining?.state === 'paused')
    return `${t('paused')} - ${app}`;
  if (!['watching', 'awaiting_progress', 'manual_watching'].includes(data.mining?.state ?? ''))
    return `${t('tab_idle')} - ${app}`;
  const drop = data.current_drop;
  if (drop) {
    const minutes = drop.confirmed_minutes;
    const percent =
      drop.confirmed_at && Number.isFinite(minutes) && drop.required_minutes > 0
        ? `${Math.floor(Math.min(100, Math.max(0, (minutes! * 100) / drop.required_minutes)))}% `
        : '';
    return `${percent}${drop.game_name} - ${app}`;
  }
  const channel = data.channels.find((channel) => channel.watching);
  return channel ? `${t('tab_watching')} ${channel.name} - ${app}` : `${t('tab_idle')} - ${app}`;
}
