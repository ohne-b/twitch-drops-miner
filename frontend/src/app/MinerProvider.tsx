import { isDesktop } from '../shared/lib/platform';
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  type ReactNode,
} from 'react';
import { accountRequest } from './accounts';
import type { Snapshot } from '../shared/lib/types';
import { connectBrowser, type Subscription } from './transport';
import { I18n } from '../shared/lib/i18n';
import { useAutosave } from './useAutosave';
import { useProgressCountdown } from './useProgressCountdown';
import { initialState, reducer } from './reducer';

export function upsert<T extends { id: string | number }>(items: T[], item: T): T[] {
  return items.some((current) => current.id === item.id)
    ? items.map((current) => (current.id === item.id ? item : current))
    : [...items, item];
}
const Context = createContext<{
  request: ReturnType<typeof accountRequest>;
  accountId: string | null;
  data: Snapshot | null;
  connected: boolean;
  incompatible: boolean;
  historyRevision: number;
  autosave: ReturnType<typeof useAutosave>;
  countdown: ReturnType<typeof useProgressCountdown>;
}>({
  request: accountRequest(null),
  accountId: null,
  data: null,
  connected: false,
  incompatible: false,
  historyRevision: 0,
  autosave: null as unknown as ReturnType<typeof useAutosave>,
  countdown: null,
});

export function MinerProvider({
  children,
  accountId,
  available,
}: {
  children: ReactNode;
  accountId: string | null;
  available: boolean;
}) {
  const request = useMemo(() => accountRequest(accountId), [accountId]);
  const [state, dispatch] = useReducer(reducer, initialState);
  const transport = useRef<Subscription | null>(null);
  const autosave = useAutosave(
    state.data?.settings,
    state.hydrated,
    (settings, revision) => {
      dispatch({ type: 'settings', settings, revision });
    },
    request,
    accountId ?? 'none',
  );
  const countdown = useProgressCountdown(state.data, state.hydrated);
  useEffect(() => {
    if (state.resync) transport.current?.resync();
  }, [state.resync]);
  useEffect(() => {
    let disposed = false;
    const start = async () => {
      if (!accountId || !available) return;
      const connect = isDesktop()
        ? (await import('../shared/lib/desktop')).connectDesktop
        : connectBrowser;
      if (disposed) return;
      transport.current = connect((action) => {
        if (
          (action.type === 'snapshot' || action.type === 'patch') &&
          action.value.account_id !== accountId
        )
          return;
        dispatch(action);
      }, accountId);
    };
    void start();
    return () => {
      disposed = true;
      transport.current?.close();
      transport.current = null;
    };
  }, [accountId, available]);
  return (
    <Context
      value={{
        request,
        accountId,
        data: state.data,
        connected: state.hydrated,
        incompatible: state.incompatible,
        historyRevision: state.data?.history_clear_revision ?? 0,
        autosave,
        countdown,
      }}
    >
      <I18n>{children}</I18n>
    </Context>
  );
}
export const useMiner = () => useContext(Context);
