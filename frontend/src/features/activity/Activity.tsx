import { useEffect, useRef, useState } from 'react';
import { Icon } from '@mdi/react';
import { CampaignLink } from '../campaigns/CampaignLink';
import {
  mdiArrowDown,
  mdiFilterOutline,
  mdiAlertCircleOutline,
  mdiDockRight,
  mdiCheck,
  mdiInformationOutline,
} from '@mdi/js';
import { useMiner } from '../../app/MinerProvider';
import { plainText, useT } from '../../shared/lib/i18n';
import { IconButton, Empty, Search, dateTime } from '../../shared/ui/index';

export default function Activity() {
  const { data } = useMiner();
  const t = useT();
  const [search, setSearch] = useState('');
  const [category, setCategory] = useState('all');
  const [severity, setSeverity] = useState('all');
  const [following, setFollowing] = useState(true);
  const ref = useRef<HTMLDivElement>(null);
  const events = (data?.activity ?? []).filter(
    (event) =>
      (category === 'all' || category === event.category) &&
      (severity === 'all' || severity === event.severity) &&
      event.message.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
  );
  useEffect(() => {
    if (following && ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [data?.activity, search, category, severity, following]);
  return (
    <div className="activity-workspace">
      <header className="flex items-center gap-3">
        <h1 className="text-[22px] font-semibold">{t('activity')}</h1>
      </header>
      <div className="flex gap-2">
        <div className="min-w-0 flex-1">
          <Search value={search} onChange={setSearch} label={t('search_activity')} />
        </div>
        <div
          className="icon-button"
          title={`${t('activity_category')}: ${t(`activity_${category}`)}`}
        >
          <Icon path={mdiFilterOutline} className="mdi-icon pointer-events-none" />
          <select
            className="icon-select absolute inset-0 size-full opacity-0"
            aria-label={t('activity_category')}
            value={category}
            onChange={(event) => setCategory(event.target.value)}
          >
            {['all', 'mining', 'claims', 'inventory', 'account', 'connection'].map((value) => (
              <option key={value} value={value}>
                {t(`activity_${value}`)}
              </option>
            ))}
          </select>
        </div>
        <div
          className="icon-button"
          title={`${t('activity_severity')}: ${t(`activity_${severity}`)}`}
        >
          <Icon path={mdiAlertCircleOutline} className="mdi-icon pointer-events-none" />
          <select
            className="icon-select absolute inset-0 size-full opacity-0"
            aria-label={t('activity_severity')}
            value={severity}
            onChange={(event) => setSeverity(event.target.value)}
          >
            {['all', 'info', 'warning', 'error'].map((value) => (
              <option key={value} value={value}>
                {t(`activity_${value}`)}
              </option>
            ))}
          </select>
        </div>
        <IconButton
          path={mdiArrowDown}
          label={t('follow_activity')}
          aria-pressed={following}
          onClick={() => setFollowing(true)}
        />
      </div>
      <div
        ref={ref}
        id="activity-list"
        data-restore-scroll
        tabIndex={0}
        aria-label={t('activity')}
        className="panel activity-list overflow-y-auto"
        onScroll={(event) => {
          const element = event.currentTarget;
          setFollowing(element.scrollHeight - element.scrollTop - element.clientHeight < 40);
        }}
      >
        {events.map((event) => (
          <article key={event.id} className="activity-row">
            <time
              className="text-xs tabular-nums text-muted"
              dateTime={event.last_at}
              title={dateTime(event.last_at)}
            >
              {new Date(event.last_at).toLocaleTimeString([], {
                hour: '2-digit',
                minute: '2-digit',
                second: '2-digit',
              })}
            </time>
            <Icon
              className="mdi-icon text-muted"
              path={
                event.recovered
                  ? mdiCheck
                  : event.severity === 'info'
                    ? mdiInformationOutline
                    : mdiAlertCircleOutline
              }
              title={t(event.recovered ? 'activity_recovered' : `activity_${event.severity}`)}
            />
            <div className="min-w-0 flex-1">
              <p className="text-[13px] text-soft">{plainText(event.message)}</p>
              {event.recovered && <p className="muted mt-1 text-xs">{t('activity_recovered')}</p>}
            </div>
            {event.count > 1 && (
              <span
                className="muted text-xs tabular-nums"
                title={t('activity_repeated', {
                  count: event.count,
                  first: dateTime(event.first_at),
                  last: dateTime(event.last_at),
                })}
              >
                ×{event.count}
              </span>
            )}
            {event.campaign_id && (
              <CampaignLink
                id={`activity-campaign-${event.id}`}
                className="icon-button"
                aria-label={t('campaign_details')}
                title={t('campaign_details')}
                to={`/campaigns?campaign=${encodeURIComponent(event.campaign_id)}${event.drop_id ? `&drop=${encodeURIComponent(event.drop_id)}` : ''}`}
              >
                <Icon path={mdiDockRight} className="mdi-icon" />
              </CampaignLink>
            )}
          </article>
        ))}
        {!events.length && (
          <Empty title={t(data?.activity.length ? 'no_activity' : 'activity_empty')} />
        )}
      </div>
    </div>
  );
}
