import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useT } from '../../shared/lib/i18n';
import { Button, Dialog, Notice, ProgressBar } from '../../shared/ui';

export type UpdateStatus = {
  revision: number;
  phase:
    | 'idle'
    | 'checking'
    | 'current'
    | 'available'
    | 'downloading'
    | 'ready'
    | 'installing'
    | 'failed';
  current_version: string;
  version: string | null;
  downloaded: number;
  total: number | null;
  error: string | null;
  restart_required: boolean;
};

export function newerUpdate(current: UpdateStatus | null, next: UpdateStatus) {
  return current && current.revision > next.revision ? current : next;
}

export function DesktopUpdateButton() {
  const t = useT();
  return (
    <Button onClick={() => window.dispatchEvent(new Event('desktop-update-open'))}>
      {t('gui.desktop.check_updates')}
    </Button>
  );
}

export default function DesktopUpdates() {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [error, setError] = useState(false);
  const accept = (value: UpdateStatus) => setStatus((current) => newerUpdate(current, value));
  async function action(action?: 'check' | 'download' | 'cancel' | 'install') {
    setError(false);
    try {
      accept(await invoke('desktop_update', { action }));
    } catch {
      setError(true);
    }
  }
  useEffect(() => {
    let disposed = false;
    const show = () => {
      setOpen(true);
      setError(false);
      void invoke<UpdateStatus>('desktop_update')
        .then(async (current) => {
          if (disposed) return;
          accept(current);
          if (
            current.phase === 'idle' ||
            current.phase === 'current' ||
            (current.phase === 'failed' && current.error === 'check_failed')
          )
            await action('check');
        })
        .catch(() => {
          if (!disposed) setError(true);
        });
    };
    const statusListener = listen<UpdateStatus>('desktop-update', ({ payload }) => {
      if (!disposed) accept(payload);
    });
    const openListener = listen('desktop-update-open', show);
    window.addEventListener('desktop-update-open', show);
    // Checking is quiet; installation always needs an explicit click.
    void statusListener
      .then(async () => {
        if (disposed) return;
        const initial = await invoke<UpdateStatus>('desktop_update');
        if (disposed) return;
        accept(initial);
        if (initial.phase === 'idle') await action('check');
      })
      .catch(() => {
        if (!disposed) setError(true);
      });
    return () => {
      disposed = true;
      window.removeEventListener('desktop-update-open', show);
      void statusListener.then((stop) => stop());
      void openListener.then((stop) => stop());
    };
  }, []);
  const busy =
    status?.phase === 'checking' ||
    status?.phase === 'downloading' ||
    status?.phase === 'installing';
  return (
    <>
      {status?.version && (
        <div
          className="desktop-update-link flex justify-end px-4 py-2 lg:justify-start lg:pt-4 lg:pb-0"
          aria-live="polite"
        >
          <button
            type="button"
            className="min-h-9 max-w-full truncate rounded px-1 text-start text-xs text-muted hover:text-text max-lg:min-h-11"
            aria-haspopup="dialog"
            title={t('gui.desktop.update_available', { version: status.version })}
            onClick={() => setOpen(true)}
          >
            {t('gui.desktop.update_link', { version: status.version })}
          </button>
        </div>
      )}
      <Dialog
        open={open}
        title={t('gui.desktop.updates')}
        onClose={() => {
          if (status?.phase !== 'installing') setOpen(false);
        }}
      >
        <div className="space-y-4">
          <p className="muted">Drops Miner {status?.current_version}</p>
          {status && (
            <p role="status">
              {t(`gui.desktop.update_${status.phase}`, { version: status.version ?? '' })}
            </p>
          )}
          {status?.phase === 'downloading' &&
            (status.total ? (
              <ProgressBar
                current={status.downloaded}
                total={status.total}
                label={t('gui.desktop.download')}
              />
            ) : (
              <p className="muted">{Math.floor(status.downloaded / (1024 * 1024))} MB</p>
            ))}
          {status?.version && (
            <a
              className="text-link inline-block"
              href={`https://github.com/ohne-b/twitch-drops-miner/releases/tag/v${encodeURIComponent(status.version)}`}
              target="_blank"
              rel="noreferrer"
            >
              {t('release_notes')}
            </a>
          )}
          {(error || status?.error) && (
            <Notice error>{t(`gui.desktop.${status?.error ?? 'error'}`)}</Notice>
          )}
          <div className="flex flex-wrap justify-end gap-2">
            {status?.restart_required ? (
              <Button
                primary
                onClick={() => void invoke('restart_app').catch(() => setError(true))}
              >
                {t('gui.desktop.restart')}
              </Button>
            ) : status?.phase === 'ready' ? (
              <Button primary onClick={() => void action('install')}>
                {t('gui.desktop.install')}
              </Button>
            ) : status?.phase === 'available' ||
              (status?.phase === 'failed' && status.error === 'download_failed') ? (
              <Button primary onClick={() => void action('download')}>
                {t('gui.desktop.download')}
              </Button>
            ) : (
              !busy && (
                <Button onClick={() => void action('check')}>
                  {t('gui.desktop.check_updates')}
                </Button>
              )
            )}
            {(status?.phase === 'downloading' || status?.phase === 'checking') && (
              <Button onClick={() => void action('cancel')}>{t('cancel')}</Button>
            )}
            {status?.phase !== 'installing' && (
              <Button onClick={() => setOpen(false)}>{t('close')}</Button>
            )}
          </div>
        </div>
      </Dialog>
    </>
  );
}
