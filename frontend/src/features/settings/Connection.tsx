import type { ActionState } from '../../shared/ui';
import { useEffect, useState } from 'react';
import { Icon } from '@mdi/react';
import { mdiContentSaveOutline, mdiReload, mdiTrashCanOutline } from '@mdi/js';
import { useAccounts, useSharedState } from '../../app/accounts';
import { request } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import type { Result } from '../../shared/lib/types';
import { ActionResult, Button, Field, IconButton, Input, Notice, useAction } from '../../shared/ui';

interface Values {
  catalog_api_key?: string;
  proxy: string;
  connection_quality: number;
  minimum_refresh_interval_minutes: number;
}
interface Saved {
  catalog_api_key_configured: boolean;
  revision: string;
  values: Values;
}

export default function Connection() {
  const { connected, action } = useAccounts();
  const t = useT();
  const [saved, setSaved] = useSharedState<Saved | null>('connection.saved', null);
  const [draft, setDraft] = useSharedState<Values | null>('connection.draft', null);
  const [loading, setLoading] = useState(!saved);
  const [failed, setFailed] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const command = useAction(
    useSharedState<ActionState>('connection.action', { busy: false, error: '', success: '' }),
  );
  const proxy = useAction();
  const [attempt, retry] = useState(0);
  useEffect(() => {
    if (saved) return;
    const controller = new AbortController();
    setLoading(true);
    setFailed(false);
    void request<Saved>('/api/connection', undefined, 'GET', controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) {
          setSaved(value);
          setDraft(value.values);
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) setFailed(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [attempt, !!saved]);
  const dirty = !!saved && !!draft && JSON.stringify(saved.values) !== JSON.stringify(draft);
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (dirty) event.preventDefault();
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);
  if (!draft || !saved)
    return loading ? (
      <p role="status" className="text-muted">
        {t('loading')}
      </p>
    ) : failed ? (
      <Notice error>
        {t('request_failed')}
        <Button onClick={() => retry((value) => value + 1)}>{t('retry')}</Button>
      </Notice>
    ) : null;
  function change<K extends keyof Values>(key: K, value: Values[K]) {
    setDraft((previous) => previous && { ...previous, [key]: value });
    command.clear();
  }
  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        void command.run(async () => {
          const result = await action<Saved & { unavailable: string[] }>({
            action: 'connection',
            revision: saved.revision,
            values: draft,
          });
          setSaved(result);
          setDraft(result.values);
          setUnavailable(!!result.unavailable?.length);
        });
      }}
    >
      <fieldset disabled={!connected || command.busy} className="space-y-4">
        <Field label={t('gui.settings.minimum_refresh')} detail={t('inventory_refresh_help')}>
          <Input
            type="number"
            min={1}
            max={1440}
            step={1}
            required
            value={
              Number.isFinite(draft.minimum_refresh_interval_minutes)
                ? draft.minimum_refresh_interval_minutes
                : ''
            }
            onChange={(event) =>
              change('minimum_refresh_interval_minutes', event.target.valueAsNumber)
            }
          />
        </Field>
        <div className="grid gap-4 sm:grid-cols-2 sm:[&>div>div:first-child]:min-h-11 md:[&>div>div:first-child]:min-h-9">
          <Field label={t('proxy')} detail={t('proxy_help')}>
            <Input
              type="url"
              maxLength={4096}
              autoComplete="off"
              value={draft.proxy}
              placeholder="http://127.0.0.1:8080"
              onChange={(event) => change('proxy', event.target.value)}
            />
          </Field>
          <Field label={t('gui.settings.connection_quality')}>
            <select
              className="field"
              value={draft.connection_quality}
              onChange={(event) => change('connection_quality', Number(event.target.value))}
            >
              {[1, 2, 3, 4, 5, 6].map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <div className="grid grid-cols-[1fr_auto] items-end gap-2">
          <Field label={t('catalog_api_key')} detail={t('catalog_api_key_help')}>
            <Input
              type="password"
              autoComplete="off"
              maxLength={512}
              value={draft.catalog_api_key ?? ''}
              placeholder={
                draft.catalog_api_key === ''
                  ? t('catalog_api_key_removed')
                  : saved.catalog_api_key_configured
                    ? t('catalog_api_key_saved')
                    : ''
              }
              onChange={(event) => change('catalog_api_key', event.target.value || undefined)}
            />
          </Field>
          <IconButton
            path={mdiTrashCanOutline}
            label={t('catalog_api_key_remove')}
            disabled={
              draft.catalog_api_key === '' ||
              (!saved.catalog_api_key_configured && !draft.catalog_api_key)
            }
            onClick={() => change('catalog_api_key', '')}
          />
        </div>
        <div className="flex flex-wrap gap-2">
          <Button primary type="submit" disabled={!dirty}>
            <Icon path={mdiContentSaveOutline} className="mdi-icon" />
            {t('accounts_save')}
          </Button>
          <Button
            disabled={!draft.proxy || proxy.busy}
            onClick={() =>
              void proxy.run(async () => {
                const result = await request<Result>('/api/settings/verify-proxy', {
                  proxy: draft.proxy,
                });
                if (!result.success) throw new Error();
              }, t('connection_verified'))
            }
          >
            {t('verify_proxy')}
          </Button>
          {command.error && (
            <Button
              onClick={() => {
                setSaved(null);
                setDraft(null);
                command.clear();
              }}
            >
              <Icon path={mdiReload} className="mdi-icon" />
              {t('accounts_reload_connection')}
            </Button>
          )}
        </div>
      </fieldset>
      <ActionResult action={command} />
      <ActionResult action={proxy} />
      {unavailable && <Notice>{t('accounts_connection_partial')}</Notice>}
      <p className="text-xs text-muted">{t(connected ? 'connected' : 'connecting')}</p>
    </form>
  );
}
