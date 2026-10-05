import { useEffect, useId, useRef, useState } from 'react';
import { useLocation } from 'react-router';
import { Icon } from '@mdi/react';
import { mdiClose, mdiOpenInNew, mdiAccountOutline } from '@mdi/js';
import type { AccountBadge, AccountProfile } from '../../shared/lib/types';
import { safeUrl } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import { Art, IconButton, dateTime, dateOnly } from '../../shared/ui';
import { readableNameColor } from './profile';

function Avatar({ profile, large = false }: { profile?: AccountProfile; large?: boolean }) {
  return profile?.avatar_url ? (
    <Art
      url={profile.avatar_url}
      className={large ? 'size-16! rounded-full!' : 'size-9! rounded-full!'}
    />
  ) : (
    <span
      className={`grid shrink-0 place-items-center rounded-full bg-raised text-muted ${large ? 'size-16' : 'size-9'}`}
    >
      <Icon path={mdiAccountOutline} className={large ? 'size-8' : 'size-5'} />
    </span>
  );
}

export default function Account({
  profile,
  userId,
  compact = false,
}: {
  profile?: AccountProfile;
  userId: number;
  compact?: boolean;
}) {
  const t = useT();
  const id = useId();
  const card = useRef<HTMLDivElement>(null);
  const location = useLocation();
  const [selected, setSelected] = useState<string | null>(null);
  const name = profile?.display_name ?? t('account');
  const color = readableNameColor(profile?.color ?? null);
  const badges = [...(profile?.available_badges ?? [])];
  for (const badge of profile?.badges ?? [])
    if (!badges.some((existing) => existing.id === badge.id)) badges.push(badge);
  const activeBadge = badges.find((badge) => badge.id === selected);
  useEffect(() => {
    card.current?.hidePopover();
    setSelected(null);
  }, [userId, location.key]);
  useEffect(() => {
    if (!compact) return;
    const desktop = matchMedia('(min-width: 1024px)');
    const close = () => card.current?.hidePopover();
    desktop.addEventListener('change', close);
    return () => desktop.removeEventListener('change', close);
  }, [compact]);
  const badgeTitle = (badge: AccountBadge) =>
    profile?.badges.some((equipped) => equipped.id === badge.id)
      ? t('profile_equipped_badge', { name: badge.title })
      : badge.title;
  return (
    <>
      <button
        type="button"
        popoverTarget={id}
        aria-haspopup="dialog"
        aria-label={t('profile_open', { name })}
        title={t('profile_open', { name })}
        className={`account-trigger flex min-h-11 min-w-0 items-center gap-2.5 rounded text-start ${compact ? 'mt-2 w-full' : 'gap-3'}`}
      >
        <Avatar profile={profile} />
        <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
          <span className="truncate font-semibold" style={{ color }}>
            {name}
          </span>
          {compact &&
            profile?.badges.map((badge) => (
              <span key={badge.id} title={badge.title}>
                <Art
                  url={badge.image_url}
                  fit="contain"
                  className="size-[18px]! rounded-none! bg-transparent!"
                />
              </span>
            ))}
        </span>
      </button>
      <div
        ref={card}
        id={id}
        popover="auto"
        role="dialog"
        aria-labelledby={`${id}-name`}
        className={`account-card ${compact ? 'account-card-sidebar' : ''}`}
      >
        <div className="relative h-28 bg-raised">
          {profile?.banner_url && (
            <Art url={profile.banner_url} className="size-full! rounded-none!" />
          )}
          <IconButton
            path={mdiClose}
            label={t('close')}
            popoverTarget={id}
            popoverTargetAction="hide"
            className="absolute end-2 top-2 bg-surface! shadow-sm"
          />
        </div>
        <div className="relative space-y-4 px-5 pb-5">
          <div className="flex items-start gap-3">
            <div className="-mt-8 rounded-full border-4 border-surface">
              <Avatar profile={profile} large />
            </div>
            <div className="min-w-0 flex-1 space-y-1 pt-3">
              <h2 id={`${id}-name`} className="break-words text-lg font-semibold" style={{ color }}>
                {name}
              </h2>
              {profile && <p className="break-words text-xs text-muted">@{profile.login}</p>}
              {!!profile?.roles.length && (
                <p className="text-xs text-muted">
                  {profile.roles.map((role) => t(`profile_role_${role}`)).join(' · ')}
                </p>
              )}
            </div>
            {profile && (
              <a
                href={`https://www.twitch.tv/${encodeURIComponent(profile.login)}`}
                target="_blank"
                rel="noreferrer"
                className="icon-button mt-2"
                aria-label={t('profile_twitch')}
                title={t('profile_twitch')}
              >
                <Icon path={mdiOpenInNew} className="mdi-icon" />
              </a>
            )}
          </div>
          {profile?.description && (
            <p className="whitespace-pre-wrap break-words text-[13px] leading-relaxed text-soft">
              {profile.description}
            </p>
          )}
          <dl className="space-y-1.5 text-[13px]">
            {profile?.created_at && (
              <div className="flex flex-wrap gap-x-2">
                <dt className="text-muted">{t('profile_created')}</dt>
                <dd>
                  <time dateTime={profile.created_at} title={dateTime(profile.created_at)}>
                    {dateOnly(profile.created_at)}
                  </time>
                </dd>
              </div>
            )}
            {profile?.followers != null && (
              <div className="flex gap-2">
                <dt className="text-muted">{t('profile_followers')}</dt>
                <dd>{profile.followers.toLocaleString()}</dd>
              </div>
            )}
            <div className="flex gap-2">
              <dt className="text-muted">{t('profile_id')}</dt>
              <dd className="tabular-nums">{userId}</dd>
            </div>
          </dl>
          {!!badges.length && (
            <section aria-label={t('profile_badges')} className="border-t border-divider pt-4">
              <h3 className="mb-2 text-xs text-muted">{t('profile_badges')}</h3>
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
              {activeBadge && (
                <div className="mt-3 space-y-1 text-[13px]" role="status">
                  <p className="font-medium">{badgeTitle(activeBadge)}</p>
                  {activeBadge.description && (
                    <p className="break-words leading-relaxed text-muted">
                      {activeBadge.description}
                    </p>
                  )}
                </div>
              )}
            </section>
          )}
          {profile?.available_badges == null && (
            <p className="text-xs text-muted">
              {t(profile ? 'profile_badges_unavailable' : 'profile_unavailable')}
            </p>
          )}
          {!!profile?.socials.length && (
            <div className="flex flex-wrap gap-x-4 gap-y-2 border-t border-divider pt-3 text-[13px]">
              {profile.socials
                .filter((link) => safeUrl(link.url))
                .map((link) => (
                  <a
                    key={link.url}
                    href={safeUrl(link.url)}
                    target="_blank"
                    rel="noreferrer"
                    className="underline decoration-control underline-offset-4"
                  >
                    {link.name}
                  </a>
                ))}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
