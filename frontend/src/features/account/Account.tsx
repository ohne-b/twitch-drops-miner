import { useState } from 'react';
import { Icon } from '@mdi/react';
import { mdiAccountOutline } from '@mdi/js';
import type { AccountBadge, AccountProfile } from '../../shared/lib/types';
import { useT } from '../../shared/lib/i18n';
import { Art } from '../../shared/ui';
import { readableNameColor } from './profile';

export default function AccountIdentity({
  profile,
  compact = false,
}: {
  profile?: AccountProfile;
  compact?: boolean;
}) {
  const t = useT();
  const badges = profile?.badges.map((badge) => (
    <span key={badge.id} role="img" aria-label={badge.title} title={badge.title}>
      <Art
        url={badge.image_url}
        fit="contain"
        className="size-[18px]! rounded-none! bg-transparent!"
      />
    </span>
  ));
  return (
    <span className={`account-identity flex min-w-0 items-center ${compact ? 'gap-2' : 'gap-3'}`}>
      {profile?.avatar_url ? (
        <Art
          url={profile.avatar_url}
          className={compact ? 'size-8! rounded-full!' : 'size-12! rounded-full!'}
        />
      ) : (
        <span
          className={`grid shrink-0 place-items-center rounded-full bg-raised text-muted ${compact ? 'size-8' : 'size-12'}`}
        >
          <Icon path={mdiAccountOutline} className="size-5" />
        </span>
      )}
      <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
        {compact && badges}
        <span
          className="truncate font-semibold"
          style={{ color: readableNameColor(profile?.color ?? null) }}
        >
          {profile?.display_name ?? t('account')}
        </span>
        {!compact && badges}
      </span>
    </span>
  );
}

export function AccountBadges({ profile }: { profile?: AccountProfile }) {
  const t = useT();
  const [selected, setSelected] = useState<string | null>(null);
  const badges = [...(profile?.available_badges ?? [])];
  for (const badge of profile?.badges ?? [])
    if (!badges.some((existing) => existing.id === badge.id)) badges.push(badge);
  const activeBadge = badges.find((badge) => badge.id === selected);
  const badgeTitle = (badge: AccountBadge) =>
    profile?.badges.some((equipped) => equipped.id === badge.id)
      ? t('profile_equipped_badge', { name: badge.title })
      : badge.title;
  return (
    <section aria-label={t('profile_badges')} className="max-w-md space-y-3">
      <h2 className="text-[13px] font-medium text-soft">{t('profile_badges')}</h2>
      {!!badges.length && (
        <div className="flex flex-wrap gap-1">
          {badges.map((badge) => (
            <button
              key={badge.id}
              type="button"
              className="icon-button size-8! max-md:size-11!"
              title={badgeTitle(badge)}
              aria-label={badgeTitle(badge)}
              aria-pressed={badge.id === selected}
              onClick={() => setSelected(badge.id === selected ? null : badge.id)}
            >
              <Art
                url={badge.image_url}
                fit="contain"
                className="size-6! rounded-none! bg-transparent!"
              />
            </button>
          ))}
        </div>
      )}
      {activeBadge && (
        <div className="space-y-1 text-[13px]" role="status">
          <p className="font-medium">{badgeTitle(activeBadge)}</p>
          {activeBadge.description && (
            <p className="break-words leading-relaxed text-muted">{activeBadge.description}</p>
          )}
        </div>
      )}
      {profile?.available_badges == null ? (
        <p className="text-xs text-muted">
          {t(profile ? 'profile_badges_unavailable' : 'profile_unavailable')}
        </p>
      ) : (
        !badges.length && <p className="text-xs text-muted">{t('profile_badges_empty')}</p>
      )}
    </section>
  );
}
