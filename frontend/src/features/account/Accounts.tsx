import { useEffect, useState } from 'react';
import { Icon } from '@mdi/react';
import {
  mdiAccountPlusOutline,
  mdiContentCopy,
  mdiPencil,
  mdiLogout,
  mdiDeleteOutline,
  mdiOpenInNew,
  mdiPause,
  mdiPlay,
} from '@mdi/js';
import {
  useAccounts,
  accountTarget,
  validAccountLabel,
  type AccountSummary,
  type BulkResult,
} from '../../app/accounts';
import { useMiner } from '../../app/MinerProvider';
import { ApiError } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import {
  ActionResult,
  Art,
  Button,
  Check,
  Dialog,
  Empty,
  Field,
  IconButton,
  Input,
  useAction,
} from '../../shared/ui';
import { accountName, accountStatus } from './AccountSwitcher';
import AccountIdentity, { AccountBadges } from './Account';
import AccountLogin from './AccountLogin';

export default function Accounts() {
  const { overview, selected, connected, action, select } = useAccounts();
  const { data } = useMiner();
  const accounts = overview?.accounts ?? [];
  const t = useT();
  const command = useAction();
  const [checked, setChecked] = useState<string[]>([]);
  const [result, setResult] = useState<BulkResult | null>(null);
  const [retryPause, setRetryPause] = useState<boolean | null>(null);
  const [retryCopy, setRetryCopy] = useState<{ source: string; autoClaim: boolean } | null>(null);
  const [menu, setMenu] = useState<AccountSummary | null>(null);
  const [operation, setOperation] = useState<'rename' | 'sign_out' | 'remove' | 'associate' | null>(
    null,
  );
  const [label, setLabel] = useState('');
  const [associate, setAssociate] = useState(false);
  const [loginOpen, setLoginOpen] = useState(false);
  const [copy, setCopy] = useState(false);
  const [copySource, setCopySource] = useState('');
  const [copyAutoClaim, setCopyAutoClaim] = useState(false);
  const sources = accounts.filter((account) => !account.unavailable);
  const sourceIds = sources.map((account) => account.id).join(',');
  const validSource = sources.some((account) => account.id === copySource);
  useEffect(() => {
    if (copy && !validSource) setCopySource(sources[0]?.id ?? '');
  }, [copy, copySource, sourceIds]);
  const ids = accounts.map((account) => account.id).join(',');
  useEffect(
    () =>
      setChecked((previous) =>
        previous.filter((id) => accounts.some((account) => account.id === id)),
      ),
    [ids],
  );
  const targets = accounts.filter((account) => checked.includes(account.id));
  const disabled = !connected || command.busy;
  const failures =
    result?.results.filter(
      (result) => !result.ok && accounts.some((account) => account.id === result.id),
    ) ?? [];
  async function pause(paused: boolean, ids = checked) {
    setRetryPause(paused);
    setRetryCopy(null);
    await command.run(async () =>
      setResult(
        await action<BulkResult>({
          action: 'pause',
          paused,
          targets: accounts.filter((account) => ids.includes(account.id)).map(accountTarget),
        }),
      ),
    );
  }
  async function copyPreferences(sourceId: string, autoClaim: boolean, ids = checked) {
    const source = accounts.find((account) => account.id === sourceId);
    setRetryPause(null);
    setRetryCopy({ source: sourceId, autoClaim });
    await command.run(async () => {
      if (!source || source.unavailable) throw new ApiError(409, 'accounts_unavailable');
      setResult(
        await action<BulkResult>({
          action: 'copy_preferences',
          source: accountTarget(source),
          auto_claim: autoClaim,
          targets: accounts
            .filter((account) => ids.includes(account.id) && account.id !== sourceId)
            .map(accountTarget),
        }),
      );
      setCopy(false);
    });
  }
  async function login(account?: AccountSummary) {
    if (account?.unassigned && !associate) {
      setMenu(account);
      setOperation('associate');
      return;
    }
    if (
      await command.run(() =>
        action({
          action: 'start_login',
          target: account?.id ?? null,
          associate_existing: !!account?.unassigned,
        }),
      )
    ) {
      setMenu(null);
      setOperation(null);
      setAssociate(false);
      setLoginOpen(true);
    }
  }
  function openMenu(account: AccountSummary, operation: 'rename' | 'sign_out' | 'remove') {
    command.clear();
    setMenu(account);
    setOperation(operation);
    setLabel(account.label);
    setAssociate(false);
  }
  function accountActions(account: AccountSummary) {
    return (
      <>
        {!account.connected && !account.unavailable && (
          <Button disabled={disabled} className="shrink-0" onClick={() => void login(account)}>
            {t('accounts_sign_in')}
          </Button>
        )}
        {account.unavailable && (
          <Button
            disabled={disabled}
            onClick={() => void command.run(() => action({ action: 'retry', id: account.id }))}
          >
            {t('retry')}
          </Button>
        )}
        <IconButton
          path={mdiPencil}
          label={t('accounts_rename_name', {
            name: accountName(account) || t('account'),
          })}
          disabled={disabled}
          onClick={() => openMenu(account, 'rename')}
        />
        <IconButton
          path={mdiLogout}
          label={t('accounts_sign_out_name', {
            name: accountName(account) || t('account'),
          })}
          disabled={disabled || !account.connected}
          onClick={() => openMenu(account, 'sign_out')}
        />
        <IconButton
          path={mdiDeleteOutline}
          label={t('accounts_remove_name', {
            name: accountName(account) || t('account'),
          })}
          disabled={disabled}
          onClick={() => openMenu(account, 'remove')}
        />
      </>
    );
  }
  const addAccount = (
    <IconButton
      path={mdiAccountPlusOutline}
      label={t(overview?.login ? 'accounts_continue_login' : 'accounts_add')}
      disabled={disabled}
      onClick={() => (overview?.login ? setLoginOpen(true) : void login())}
    />
  );
  return (
    <div className="space-y-4">
      {accounts.length > 1 ? (
        <div className="overflow-hidden rounded border border-divider">
          <div className="flex min-h-14 items-center gap-1 border-b border-divider px-3 py-2">
            <div className="min-w-0 flex-1">
              <label className="flex min-h-11 min-w-0 items-center gap-2 text-soft">
                <input
                  type="checkbox"
                  className="size-4 shrink-0 accent-soft"
                  checked={checked.length === accounts.length}
                  onChange={(event) =>
                    setChecked(event.target.checked ? accounts.map((account) => account.id) : [])
                  }
                />
                <span className={checked.length ? 'max-sm:sr-only' : ''}>
                  {checked.length
                    ? t('accounts_selected', { count: checked.length })
                    : t('accounts_select_all')}
                </span>
                {!!checked.length && (
                  <span className="sm:hidden" aria-hidden="true">
                    {checked.length}
                  </span>
                )}
              </label>
            </div>

            {!!targets.length && (
              <div className="flex shrink-0 items-center gap-1">
                <IconButton
                  path={mdiPause}
                  label={t('accounts_pause')}
                  disabled={disabled}
                  onClick={() => void pause(true)}
                />
                <IconButton
                  path={mdiPlay}
                  label={t('accounts_resume')}
                  disabled={disabled}
                  onClick={() => void pause(false)}
                />
                <IconButton
                  path={mdiContentCopy}
                  label={t('accounts_copy')}
                  disabled={disabled || accounts.length < 2}
                  onClick={() => {
                    command.clear();
                    setResult(null);
                    setRetryPause(null);
                    setCopySource(
                      sources.find((account) => account.id === selected)?.id ??
                        sources[0]?.id ??
                        '',
                    );
                    setCopyAutoClaim(false);
                    setCopy(true);
                  }}
                />
              </div>
            )}
            {addAccount}
          </div>
          <div
            className="max-h-[min(55dvh,540px)] overflow-y-auto overscroll-contain divide-y divide-divider"
            aria-label={t('accounts')}
          >
            {accounts.map((account) => (
              <div
                key={account.id}
                className="grid min-h-20 grid-cols-[auto_auto_minmax(0,1fr)] items-center gap-x-3 gap-y-1 px-4 py-3 sm:grid-cols-[auto_auto_minmax(0,1fr)_auto]"
              >
                <label className="flex min-h-11 shrink-0 items-center">
                  <input
                    type="checkbox"
                    className="size-4 accent-soft"
                    checked={checked.includes(account.id)}
                    onChange={(event) =>
                      setChecked((previous) =>
                        event.target.checked
                          ? [...previous, account.id]
                          : previous.filter((id) => id !== account.id),
                      )
                    }
                  />
                  <span className="sr-only">
                    {t('accounts_select', { name: accountName(account) || t('account') })}
                  </span>
                </label>
                <Art url={account.avatar_url} className="size-9! rounded-full!" />
                <div className="min-w-0 flex-1">
                  <button
                    type="button"
                    className="text-link max-w-full truncate text-start font-medium"
                    aria-current={selected === account.id ? 'true' : undefined}
                    onClick={() => select(account.id)}
                  >
                    {accountName(account) || t('account')}
                  </button>
                  <p className="truncate text-[13px] text-muted">
                    {t(accountStatus(account))}
                    {account.connected && !account.paused && account.game
                      ? `: ${account.game}`
                      : ''}
                  </p>
                  {account.connected && !account.paused && account.reward && (
                    <p className="truncate text-xs text-muted">{account.reward}</p>
                  )}
                </div>
                <div className="col-start-3 flex flex-wrap items-center justify-end gap-1 sm:col-start-auto">
                  {accountActions(account)}
                </div>
              </div>
            ))}
          </div>
        </div>
      ) : !data?.login.user_id ? (
        <div className="flex flex-wrap items-center justify-end gap-1">
          {accounts[0] && accountActions(accounts[0])}
          {addAccount}
        </div>
      ) : null}
      {!accounts.length && <Empty title={t('accounts_empty')} detail={t('accounts_empty_help')} />}
      {accounts.length === 1 && !accounts[0]!.connected && (
        <p className="text-muted text-sm">
          {accountName(accounts[0]!)}: {t(accountStatus(accounts[0]!))}
        </p>
      )}
      <ActionResult action={command} />
      {!!failures.length && (
        <div className="space-y-2" role="status">
          <ul className="space-y-1 text-[13px]">
            {failures.map((failure) => (
              <li key={failure.id}>
                {accountName(
                  accounts.find((account) => account.id === failure.id) ??
                    ({ label: failure.id } as AccountSummary),
                )}
                :{' '}
                {t(
                  failure.error === 'settings_conflict'
                    ? 'settings_conflict'
                    : 'accounts_action_failed',
                )}
              </li>
            ))}
          </ul>
          {(retryPause !== null || retryCopy) && (
            <Button
              disabled={disabled}
              onClick={() =>
                void (retryCopy
                  ? copyPreferences(
                      retryCopy.source,
                      retryCopy.autoClaim,
                      failures.map((failure) => failure.id),
                    )
                  : pause(
                      retryPause!,
                      failures.map((failure) => failure.id),
                    ))
              }
            >
              {t('accounts_retry_failed')}
            </Button>
          )}
        </div>
      )}
      {data?.login.user_id && (
        <div className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <AccountIdentity
              profile={data.login.profile}
              name={accounts.length === 1 ? accountName(accounts[0]!) : undefined}
            />
            {data.login.profile?.login && (
              <a
                className="icon-button"
                aria-label={t('open_twitch_profile')}
                title={t('open_twitch_profile')}
                href={`https://www.twitch.tv/${encodeURIComponent(data.login.profile.login)}`}
                target="_blank"
                rel="noreferrer"
              >
                <Icon path={mdiOpenInNew} className="mdi-icon" />
              </a>
            )}
            {accounts.length <= 1 && (
              <div className="ms-auto flex flex-wrap items-center gap-1">
                {accounts[0] && accountActions(accounts[0])}
                {addAccount}
              </div>
            )}
          </div>
          <AccountBadges key={data.login.user_id} profile={data.login.profile} />
        </div>
      )}
      <Dialog
        open={!!menu}
        title={menu ? accountName(menu) || t('account') : ''}
        onClose={() => {
          if (!command.busy) {
            setMenu(null);
            setOperation(null);
          }
        }}
      >
        {menu && (
          <div className="space-y-4">
            {operation === 'rename' ? (
              <Field
                label={t('accounts_label')}
                detail={t('accounts_label_help')}
                help={!validAccountLabel(label) ? t('accounts_label_invalid') : undefined}
              >
                <Input
                  value={label}
                  maxLength={100}
                  aria-invalid={!validAccountLabel(label)}
                  onChange={(event) => setLabel(event.target.value)}
                />
              </Field>
            ) : (
              <p className="text-[13px] text-muted">
                {t(
                  operation === 'associate'
                    ? 'accounts_associate_help'
                    : operation === 'sign_out'
                      ? 'accounts_sign_out_help'
                      : 'accounts_remove_help',
                )}
              </p>
            )}
            {operation === 'associate' && (
              <Check
                label={t('accounts_associate_confirm')}
                checked={associate}
                onChange={setAssociate}
              />
            )}
            <div className="flex justify-end gap-2">
              <Button
                disabled={command.busy}
                onClick={() => {
                  setMenu(null);
                  setOperation(null);
                }}
              >
                {t('cancel')}
              </Button>
              <Button
                primary
                disabled={
                  command.busy ||
                  (operation === 'associate' && !associate) ||
                  (operation === 'rename' && !validAccountLabel(label))
                }
                onClick={() =>
                  void (operation === 'associate'
                    ? login(menu)
                    : command.run(async () => {
                        await action({
                          action: operation,
                          id: menu.id,
                          ...(operation === 'rename' ? { label } : {}),
                        });
                        if (operation === 'remove' && menu.id === selected) {
                          const next = accounts.find((account) => account.id !== menu.id);
                          if (next) select(next.id);
                        }
                        setMenu(null);
                        setOperation(null);
                      }))
                }
              >
                {t(
                  operation === 'remove'
                    ? 'accounts_remove'
                    : operation === 'sign_out'
                      ? 'twitch_logout'
                      : operation === 'associate'
                        ? 'accounts_sign_in'
                        : 'accounts_save',
                )}
              </Button>
            </div>
            <ActionResult action={command} />
          </div>
        )}
      </Dialog>
      <Dialog
        open={copy}
        title={t('accounts_copy')}
        onClose={() => {
          if (!command.busy) setCopy(false);
        }}
      >
        <div className="space-y-4">
          <Field label={t('accounts_copy_from')} detail={t('accounts_copy_help')}>
            <select
              className="field"
              value={validSource ? copySource : ''}
              onChange={(event) => setCopySource(event.target.value)}
            >
              {!validSource && (
                <option value="" disabled>
                  {t('accounts_unavailable')}
                </option>
              )}
              {sources.map((account) => (
                <option key={account.id} value={account.id}>
                  {accountName(account) || t('account')}
                </option>
              ))}
            </select>
          </Field>
          <p className="text-[13px] text-muted">
            {t('accounts_copy_to', {
              names:
                targets
                  .filter((account) => account.id !== copySource)
                  .map(accountName)
                  .join(', ') || t('accounts_no_targets'),
            })}
          </p>
          <Check
            label={t('accounts_copy_auto_claim')}
            checked={copyAutoClaim}
            onChange={setCopyAutoClaim}
          />
          <ActionResult action={command} />
          <div className="flex justify-end gap-2">
            <Button disabled={command.busy} onClick={() => setCopy(false)}>
              {t('cancel')}
            </Button>
            <Button
              primary
              disabled={
                disabled || !validSource || !targets.some((account) => account.id !== copySource)
              }
              onClick={() => void copyPreferences(copySource, copyAutoClaim)}
            >
              {t('accounts_copy')}
            </Button>
          </div>
        </div>
      </Dialog>
      {loginOpen && <AccountLogin onClose={() => setLoginOpen(false)} />}
    </div>
  );
}
