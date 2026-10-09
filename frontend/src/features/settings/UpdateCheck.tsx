import { mdiUpdate } from '@mdi/js';
import type { ReactNode } from 'react';
import { useT } from '../../shared/lib/i18n';
import { IconButton } from '../../shared/ui';

export default function UpdateCheck({
  version,
  checking,
  current = false,
  disabled,
  onCheck,
  children,
}: {
  version?: string;
  checking: boolean;
  current?: boolean;
  disabled?: boolean;
  onCheck: () => void;
  children: ReactNode;
}) {
  const t = useT();
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
      {version && (
        <span
          className={`inline-flex min-h-6 max-w-full items-center rounded px-2 text-xs font-semibold wrap-anywhere ${current ? 'bg-accent/10 text-accent' : 'bg-raised text-soft'}`}
          title={t('installed_version', { version })}
        >
          v{version}
        </span>
      )}
      <IconButton
        path={mdiUpdate}
        label={t(checking ? 'checking_updates' : 'check_updates')}
        className={checking ? '[&>svg]:animate-spin motion-reduce:[&>svg]:animate-none' : ''}
        aria-busy={checking}
        disabled={disabled || checking}
        onClick={onCheck}
      />
      {children}
    </div>
  );
}
