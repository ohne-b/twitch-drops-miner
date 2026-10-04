import { Icon } from '@mdi/react';
import { useEffect, useRef, useState } from 'react';
import { Link } from 'react-router';
import { mdiArrowLeft, mdiPlus, mdiPriorityHigh } from '@mdi/js';
import type { Settings as SettingsData } from '../../shared/lib/types';
import { useMiner } from '../../app/MinerProvider';
import { useT } from '../../shared/lib/i18n';
import { GamePriorities } from './GamePriorities';
import {
  ActionResult,
  Button,
  Check,
  Dialog,
  Field,
  IconButton,
  Notice,
  Search,
  useAction,
} from '../../shared/ui/index';

export default function MiningPreferences() {
  const { data, connected, autosave } = useMiner();
  const t = useT();
  const settings = data!.settings;
  const draft = autosave.draft ?? settings;
  const change = autosave.change;
  const heading = useRef<HTMLHeadingElement>(null);
  const gameList = useRef<HTMLDivElement>(null);
  useEffect(() => {
    window.scrollTo(0, 0);
    heading.current?.focus({ preventScroll: true });
  }, []);
  const [ignoredText, setIgnoredText] = useState(draft.drop_name_blacklist.join('\n'));
  const [editingIgnored, setEditingIgnored] = useState(false);
  useEffect(() => {
    if (!editingIgnored) setIgnoredText(draft.drop_name_blacklist.join('\n'));
  }, [draft.drop_name_blacklist, editingIgnored]);
  const [search, setSearch] = useState('');
  const [gameError, setGameError] = useState('');
  const [confirmation, setConfirmation] = useState<{
    title: string;
    text: string;
    action: () => Promise<unknown>;
  } | null>(null);
  const command = useAction();
  function addGame(name: string) {
    change('games_to_watch', (games) =>
      games.some((game) => game.toLowerCase() === name.toLowerCase()) ? games : [...games, name],
    );
    setSearch('');
    setGameError('');
  }
  function resolveGame() {
    const name = search.trim();
    if (!name) return;
    const games = settings.games_available ?? [];
    const exact = games.find((item) => item.toLocaleLowerCase() === name.toLocaleLowerCase());
    const matches = games.filter((item) =>
      item.toLocaleLowerCase().includes(name.toLocaleLowerCase()),
    );
    const selected = exact ?? (matches.length === 1 ? matches[0] : undefined);
    if (selected) {
      if (!draft.games_to_watch.includes(selected)) addGame(selected);
      return;
    }
    if (matches.length > 1) {
      setGameError(t('gui.settings.multiple_games_found'));
      return;
    }
    setConfirmation({
      title: t('gui.settings.add_game'),
      text: t('gui.settings.manual_game_warning', { game: name }),
      action: async () => {
        addGame(name);
      },
    });
  }
  const available = (settings.games_available ?? []).filter(
    (game) =>
      !draft.games_to_watch.includes(game) &&
      game.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
  );
  return (
    <div className="flex flex-col gap-5 lg:min-h-0 lg:flex-1">
      <header className="flex shrink-0 items-center gap-3">
        <Link
          to="/"
          className="icon-button"
          aria-label={t('back_to_mining')}
          title={t('back_to_mining')}
        >
          <Icon path={mdiArrowLeft} className="mdi-icon" />
        </Link>
        <h1 ref={heading} tabIndex={-1} className="text-[22px] font-semibold">
          {t('mining_preferences')}
        </h1>
      </header>
      <section
        id="priorities"
        className="panel relative p-5 lg:min-h-0 lg:flex-1 lg:overflow-hidden"
        aria-label={t('mining_preferences')}
      >
        <form className="lg:h-full" onSubmit={(event) => event.preventDefault()}>
          <div className="grid min-w-0 gap-6 lg:h-full lg:grid-cols-2 lg:grid-rows-[minmax(0,1fr)]">
            <div
              role="group"
              aria-label={t('game_priorities')}
              tabIndex={!connected ? 0 : undefined}
              className="preferences-games relative flex min-h-0 min-w-0 flex-col gap-4 focus-visible:bg-field"
            >
              <div className="flex min-h-0 flex-col gap-2">
                <fieldset disabled={!connected} className="flex shrink-0 gap-2">
                  <div
                    className="min-w-0 flex-1"
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault();
                        resolveGame();
                      }
                    }}
                  >
                    <Search
                      value={search}
                      onChange={(value) => {
                        setSearch(value);
                        setGameError('');
                        gameList.current?.scrollTo(0, 0);
                      }}
                      label={t('gui.settings.search_games')}
                    />
                  </div>
                  <IconButton
                    path={mdiPlus}
                    label={t('gui.settings.add_game')}
                    onClick={resolveGame}
                    disabled={!search.trim()}
                  />
                  <div
                    className="icon-button has-[:disabled]:opacity-50"
                    title={`${t('mining_priority')}: ${t(`priority_${draft.mining_priority_mode}`)}`}
                  >
                    <Icon className="mdi-icon pointer-events-none" path={mdiPriorityHigh} />
                    <select
                      className="icon-select absolute inset-0 size-full cursor-pointer opacity-0 disabled:cursor-default"
                      aria-label={t('mining_priority')}
                      aria-describedby="mining-priority-help"
                      value={draft.mining_priority_mode}
                      onChange={(event) =>
                        change(
                          'mining_priority_mode',
                          event.target.value as SettingsData['mining_priority_mode'],
                        )
                      }
                    >
                      {(['manual', 'short_events', 'ending_soonest'] as const).map((mode) => (
                        <option key={mode} value={mode}>
                          {t(`priority_${mode}`)}
                        </option>
                      ))}
                    </select>
                  </div>
                </fieldset>
                {(gameError || (search && available.length > 0)) && (
                  <div
                    key={search}
                    role="region"
                    aria-label={t('gui.settings.search_games')}
                    tabIndex={!connected ? 0 : undefined}
                    className="scroll-list max-h-40 min-h-11 overflow-y-auto rounded border border-divider focus-visible:bg-field lg:overscroll-y-contain"
                  >
                    {gameError && <Notice error>{gameError}</Notice>}
                    <fieldset disabled={!connected}>
                      {available.map((game) => (
                        <button
                          type="button"
                          key={game}
                          className="block w-full px-3 py-2 text-start text-[13px] hover:bg-hover max-md:min-h-11"
                          onClick={() => addGame(game)}
                        >
                          {game}
                        </button>
                      ))}
                    </fieldset>
                  </div>
                )}
              </div>
              <p id="mining-priority-help" className="muted shrink-0">
                {t(
                  draft.mining_priority_mode === 'manual'
                    ? 'selected_games_help'
                    : `priority_${draft.mining_priority_mode}_help`,
                )}
              </p>
              <div
                ref={gameList}
                role="region"
                aria-label={t('game_priorities')}
                tabIndex={0}
                className="scroll-list relative max-h-[440px] min-h-0 scroll-py-1 overflow-y-auto focus-visible:bg-field focus-visible:[&_.panel]:bg-field lg:max-h-none lg:flex-1 lg:overscroll-y-contain"
              >
                <fieldset disabled={!connected}>
                  <GamePriorities
                    games={draft.games_to_watch}
                    campaigns={data?.campaigns ?? []}
                    onChange={(games) => change('games_to_watch', games)}
                    scrollContainer={gameList}
                  />
                </fieldset>
              </div>
            </div>
            <div
              role="group"
              aria-label={t('mining_preferences')}
              tabIndex={!connected ? 0 : undefined}
              className="relative min-h-0 min-w-0 focus-visible:bg-field lg:overflow-y-auto"
            >
              <fieldset disabled={!connected} className="space-y-5">
                <fieldset className="space-y-2">
                  <legend className="text-[13px] font-medium">{t('auto_mine_types')}</legend>
                  <p className="muted">{t('auto_mine_types_help')}</p>
                  <div className="flex flex-wrap gap-x-6">
                    <Check
                      label={t('auto_mine_badges')}
                      checked={draft.auto_mine_badges}
                      onChange={(value) => change('auto_mine_badges', value)}
                    />
                    <Check
                      label={t('auto_mine_emotes')}
                      checked={draft.auto_mine_emotes}
                      onChange={(value) => change('auto_mine_emotes', value)}
                    />
                  </div>
                </fieldset>
                <div>
                  <p className="mb-2 text-[13px] font-medium">
                    {t('gui.settings.mining_benefits')}
                  </p>
                  <div className="flex flex-wrap gap-x-6">
                    {[
                      ['BADGE', 'badge'],
                      ['EMOTE', 'emote'],
                      ['DIRECT_ENTITLEMENT', 'item'],
                      ['UNKNOWN', 'other'],
                    ].map(
                      ([key, label]) =>
                        key && (
                          <Check
                            key={key}
                            label={t(`gui.inventory.filters.${label}`)}
                            checked={draft.mining_benefits[key] ?? true}
                            onChange={(value) =>
                              change('mining_benefits', { ...draft.mining_benefits, [key]: value })
                            }
                          />
                        ),
                    )}
                  </div>
                </div>
                <Field
                  label={t('gui.settings.drop_name_blacklist')}
                  help={t('gui.settings.drop_name_blacklist_help')}
                >
                  <textarea
                    className="field"
                    value={ignoredText}
                    onFocus={() => setEditingIgnored(true)}
                    onBlur={() => setEditingIgnored(false)}
                    onChange={(event) => {
                      setIgnoredText(event.target.value);
                      change('drop_name_blacklist', event.target.value.split('\n'));
                    }}
                  />
                </Field>
              </fieldset>
            </div>
          </div>
        </form>
      </section>
      <Dialog
        open={confirmation !== null}
        title={confirmation?.title ?? ''}
        onClose={() => setConfirmation(null)}
      >
        <p className="text-muted">{confirmation?.text}</p>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setConfirmation(null)}>{t('cancel')}</Button>
          <Button
            primary
            disabled={command.busy}
            onClick={() =>
              void command.run(async () => {
                await confirmation?.action();
                setConfirmation(null);
              })
            }
          >
            {t('gui.settings.confirm_btn')}
          </Button>
        </div>
        <ActionResult action={command} />
      </Dialog>
    </div>
  );
}
