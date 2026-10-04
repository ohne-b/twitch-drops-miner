import { Icon } from '@mdi/react';
import { mdiDockRight } from '@mdi/js';
import type { ReactNode } from 'react';
import type { Campaign as CampaignData } from '../../shared/lib/types';
import { useT } from '../../shared/lib/i18n';
import { Art, dateTime } from '../../shared/ui/index';

export function Campaign({
  campaign,
  action,
  onOpen,
  selected,
}: {
  campaign: CampaignData;
  action?: ReactNode;
  onOpen: () => void;
  selected: boolean;
}) {
  const t = useT();
  return (
    <CampaignSummary
      id={campaign.id}
      name={campaign.name}
      game={campaign.game_name}
      image={campaign.game_box_art_url}
      time={t(campaign.upcoming ? 'gui.inventory.starts' : 'gui.inventory.ends', {
        time: dateTime(campaign.upcoming ? campaign.starts_at : campaign.ends_at),
      })}
      count={`${campaign.claimed_drops} / ${campaign.total_drops}`}
      status={t(
        `gui.inventory.status.${campaign.expired ? 'expired' : campaign.upcoming ? 'upcoming' : 'active'}`,
      )}
      onOpen={onOpen}
      selected={selected}
      action={action}
    />
  );
}

export function CampaignSummary({
  id,
  name,
  game,
  image,
  imageFit,
  time,
  count,
  status,
  action,
  onOpen,
  selected,
}: {
  id: string;
  name: string;
  game: string;
  image?: string | null;
  imageFit?: 'cover' | 'contain';
  time: string;
  count: string;
  status?: string;
  action?: ReactNode;
  onOpen: () => void;
  selected: boolean;
}) {
  const t = useT();
  return (
    <article className={`campaign-summary panel ${selected ? 'selected' : ''}`}>
      <button
        type="button"
        id={`campaign-open-${id}`}
        onClick={onOpen}
        className="campaign-open"
        aria-label={t('inspect_campaign', { campaign: name })}
        title={t('campaign_details')}
        aria-current={selected ? 'true' : undefined}
        aria-controls={selected ? 'campaign-details' : undefined}
      >
        <Art url={image} className="size-12" fit={imageFit} />
        <span className="campaign-info min-w-0 flex-1 text-start">
          <span className="campaign-title block font-medium text-text">{name}</span>
          <span className="muted mt-1 block">{game}</span>
          <span className="muted mt-1 block text-xs">{time}</span>
        </span>
        <span className="campaign-count shrink-0 text-end text-[13px]">
          <span className="block text-soft tabular-nums">{count}</span>
          {status && <span className="muted block">{status}</span>}
        </span>
        <span className="campaign-detail-icon" aria-hidden="true">
          <Icon path={mdiDockRight} className="mdi-icon" />
        </span>
      </button>
      {action && <div className="campaign-action">{action}</div>}
    </article>
  );
}
