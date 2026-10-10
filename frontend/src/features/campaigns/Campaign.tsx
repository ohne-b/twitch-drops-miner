import { mdiDockRight } from '@mdi/js';
import type { ReactNode } from 'react';
import type { Campaign as CampaignData } from '../../shared/lib/types';
import { useT } from '../../shared/lib/i18n';
import { Art, IconButton, dateTime } from '../../shared/ui/index';

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
      time={t(campaign.upcoming ? 'campaign_starts' : 'campaign_ends', {
        time: dateTime(campaign.upcoming ? campaign.starts_at : campaign.ends_at, true),
      })}
      count={`${campaign.claimed_drops}/${campaign.total_drops}`}
      countLabel={t('campaign_claimed_count', {
        claimed: campaign.claimed_drops,
        total: campaign.total_drops,
      })}
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
  countLabel,
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
  countLabel?: string;
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
      </button>
      <div className="campaign-summary-controls">
        <span className="campaign-count shrink-0 text-end text-[13px]" title={countLabel}>
          {countLabel && <span className="sr-only">{countLabel}</span>}
          <span
            className="block text-soft tabular-nums"
            aria-hidden={countLabel ? true : undefined}
          >
            {count}
          </span>
        </span>
        <IconButton
          id={`campaign-detail-${id}`}
          path={mdiDockRight}
          label={t('campaign_details')}
          className="campaign-detail-icon"
          onClick={onOpen}
          aria-current={selected ? 'true' : undefined}
          aria-controls={selected ? 'campaign-details' : undefined}
        />
        {action && <div className="campaign-action">{action}</div>}
      </div>
    </article>
  );
}
