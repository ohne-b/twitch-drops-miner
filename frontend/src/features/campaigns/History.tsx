import { useEffect, useRef, useState } from 'react';
import { useMiner } from '../../app/MinerProvider';
import { useT } from '../../shared/lib/i18n';
import { request } from '../../shared/lib/api';
import type { Campaign, HistoryEntry } from '../../shared/lib/types';
import type { CampaignSort } from './Campaigns';
import { dateTime } from '../../shared/ui/index';
import { CampaignSummary } from './Campaign';

export function useHistory(active: boolean) {
  const { connected, data, historyRevision } = useMiner();
  const [loaded, setLoaded] = useState<{
    entries: HistoryEntry[];
    instance: string;
    clear_revision: number;
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [retry, setRetry] = useState(0);
  const claimed = data?.campaigns.reduce((sum, campaign) => sum + campaign.claimed_drops, 0);
  const checked = data?.inventory_status?.checked_at;
  const user = data?.login.user_id;
  const expected = useRef({
    instance: data?.instance,
    revision: data?.history_revision ?? 0,
    clear: historyRevision,
  });
  expected.current = {
    instance: data?.instance,
    revision: data?.history_revision ?? 0,
    clear: historyRevision,
  };
  const entries =
    loaded?.instance === data?.instance && loaded?.clear_revision === historyRevision
      ? loaded.entries
      : [];
  useEffect(() => {
    if (!active || !connected) return;
    const controller = new AbortController();
    setLoading(true);
    setError(false);
    request<{
      entries: HistoryEntry[];
      instance: string;
      revision: number;
      clear_revision: number;
    }>('/api/history', undefined, 'GET', controller.signal)
      .then((result) => {
        if (
          !controller.signal.aborted &&
          result.instance === expected.current.instance &&
          result.revision >= expected.current.revision &&
          result.clear_revision === expected.current.clear
        )
          setLoaded(result);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [
    active,
    connected,
    claimed,
    checked,
    user,
    historyRevision,
    data?.history_revision,
    data?.instance,
    retry,
  ]);
  return { entries, loading, error, retry: () => setRetry((value) => value + 1) };
}

export interface HistoryCampaign {
  id: string;
  name: string;
  game: string;
  entries: HistoryEntry[];
  metadata?: Campaign;
}
export function groupHistory(entries: HistoryEntry[], campaigns: Campaign[]): HistoryCampaign[] {
  const metadata = new Map(campaigns.map((campaign) => [campaign.id, campaign]));
  const groups = new Map<string, HistoryCampaign>();
  for (const entry of entries) {
    const id = entry.campaign_id || JSON.stringify([entry.game, entry.campaign]);
    let group = groups.get(id);
    if (!group) {
      group = {
        id,
        name: entry.campaign,
        game: entry.game,
        entries: [],
        metadata: metadata.get(entry.campaign_id),
      };
      groups.set(id, group);
    }
    group.entries.push(entry);
  }
  for (const group of groups.values()) {
    group.entries.sort(
      (a, b) => Date.parse(b.claimed_at) - Date.parse(a.claimed_at) || a.id.localeCompare(b.id),
    );
  }
  return [...groups.values()];
}
export function matchesHistory(group: HistoryCampaign, games: string[], search: string): boolean {
  return (
    (!games.length ||
      games.some((game) => game.toLocaleLowerCase() === group.game.toLocaleLowerCase())) &&
    `${group.name} ${group.game} ${group.entries.map((entry) => `${entry.drop_name} ${entry.benefits.join(' ')}`).join(' ')}`
      .toLocaleLowerCase()
      .includes(search.toLocaleLowerCase())
  );
}
export function historyOrder(a: HistoryCampaign, b: HistoryCampaign, sort: CampaignSort): number {
  const difference =
    sort === 'newest' || sort === 'ending'
      ? a.metadata && b.metadata
        ? sort === 'newest'
          ? Date.parse(b.metadata.starts_at) - Date.parse(a.metadata.starts_at)
          : Date.parse(a.metadata.ends_at) - Date.parse(b.metadata.ends_at)
        : Number(!!b.metadata) - Number(!!a.metadata)
      : sort === 'drops'
        ? b.entries.length - a.entries.length
        : sort === 'name'
          ? a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true })
          : 0;
  return (
    difference ||
    Date.parse(b.entries[0]!.claimed_at) - Date.parse(a.entries[0]!.claimed_at) ||
    a.id.localeCompare(b.id)
  );
}

export default function History({
  groups,
  onOpen,
  selected,
}: {
  groups: HistoryCampaign[];
  onOpen: (id: string, trigger: string) => void;
  selected: string | null;
}) {
  const t = useT();
  return (
    <>
      {groups.map((group) => (
        <CampaignSummary
          key={group.id}
          id={group.id}
          name={group.name}
          game={group.game}
          image={group.metadata?.game_box_art_url || group.entries[0]?.image_url}
          imageFit={group.metadata?.game_box_art_url ? 'cover' : 'contain'}
          time={dateTime(group.entries[0]?.claimed_at ?? '')}
          count={t('recorded_claims', { count: group.entries.length })}
          onOpen={(trigger) => onOpen(group.id, trigger)}
          selected={selected === group.id}
        />
      ))}
    </>
  );
}
