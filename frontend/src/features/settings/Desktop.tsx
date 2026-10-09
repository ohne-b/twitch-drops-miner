import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { mdiFolderOutline } from '@mdi/js';
import { Icon } from '@mdi/react';
import { useT } from '../../shared/lib/i18n';
import { ActionResult, Button, Check, Empty, Notice, useAction } from '../../shared/ui';

type Preferences = {
  close_to_tray: boolean;
  start_minimized: boolean;
  autostart: boolean;
  notifications: boolean;
  keep_awake: boolean;
  keep_awake_failed: boolean;
  tray_available: boolean;
  version: string;
};

export function DesktopSettings() {
  const [settings, setSettings] = useState<Preferences | null>(null);
  const action = useAction();
  const t = useT();
  const refresh = () => action.run(async () => setSettings(await invoke('desktop_settings')));
  useEffect(() => {
    void refresh();
  }, []);
  return (
    <div className="space-y-3">
      {settings &&
        (
          ['autostart', 'start_minimized', 'close_to_tray', 'keep_awake', 'notifications'] as const
        ).map((key) => (
          <Check
            key={key}
            label={t(`gui.desktop.${key}`)}
            checked={settings[key]}
            disabled={
              action.busy ||
              (!settings.tray_available && (key === 'start_minimized' || key === 'close_to_tray'))
            }
            onChange={(value) =>
              void action.run(async () =>
                setSettings(await invoke('desktop_settings', { change: { [key]: value } })),
              )
            }
          />
        ))}
      {settings && !settings.tray_available && (
        <p className="muted">{t('gui.desktop.tray_unavailable')}</p>
      )}
      <ActionResult action={action} />
      {!settings && (
        <Button disabled={action.busy} onClick={() => void refresh()}>
          {t('retry')}
        </Button>
      )}
      <div className="flex flex-wrap gap-2 pt-2">
        <Button
          title={t('gui.desktop.open_data')}
          aria-label={t('gui.desktop.open_data')}
          onClick={() => void action.run(() => invoke('open_app_folder', { folder: 'data' }))}
        >
          <Icon path={mdiFolderOutline} className="mdi-icon" />
          {t('gui.desktop.data')}
        </Button>
        <Button
          title={t('gui.desktop.open_logs')}
          aria-label={t('gui.desktop.open_logs')}
          onClick={() => void action.run(() => invoke('open_app_folder', { folder: 'logs' }))}
        >
          <Icon path={mdiFolderOutline} className="mdi-icon" />
          {t('gui.desktop.logs')}
        </Button>
      </div>
    </div>
  );
}

export function DesktopStatus() {
  const [failed, setFailed] = useState(false);
  const [keepAwakeFailed, setKeepAwakeFailed] = useState(false);
  const t = useT();
  useEffect(() => {
    let disposed = false;
    let receivedPowerStatus = false;
    const listener = listen('desktop-error', () => setFailed(true));
    const powerListener = listen<boolean>('desktop-keep-awake-failed', ({ payload }) => {
      receivedPowerStatus = true;
      if (!disposed) setKeepAwakeFailed(payload);
    });
    void powerListener
      .then(() => invoke<Preferences>('desktop_settings'))
      .then((settings) => {
        if (!disposed && !receivedPowerStatus) setKeepAwakeFailed(settings.keep_awake_failed);
      })
      .catch(() => {});
    return () => {
      disposed = true;
      void listener.then((stop) => stop());
      void powerListener.then((stop) => stop());
    };
  }, []);
  return (
    (failed || keepAwakeFailed) && (
      <div className="mb-4">
        <Notice error>
          {t(keepAwakeFailed ? 'gui.desktop.keep_awake_failed' : 'gui.desktop.error')}{' '}
          <Button
            onClick={() => {
              setFailed(false);
              setKeepAwakeFailed(false);
            }}
          >
            {t('close')}
          </Button>
        </Notice>
      </div>
    )
  );
}

export function DesktopStartupError() {
  const t = useT();
  const action = useAction();
  return (
    <div className="m-auto max-w-xl p-8">
      <Empty title={t('gui.desktop.start_failed')} detail={t('gui.desktop.start_help')}>
        <div className="flex flex-wrap justify-center gap-2">
          <Button
            onClick={() => void action.run(() => invoke('open_app_folder', { folder: 'data' }))}
          >
            {t('gui.desktop.open_data')}
          </Button>
          <Button onClick={() => void action.run(() => invoke('restart_app'))}>
            {t('gui.desktop.restart')}
          </Button>
          <Button onClick={() => void action.run(() => invoke('quit_app'))}>
            {t('gui.desktop.quit')}
          </Button>
        </div>
      </Empty>
      <ActionResult action={action} />
    </div>
  );
}
