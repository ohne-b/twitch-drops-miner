import { pageScrollElement } from '../shared/ui/pageScroll';
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type Dispatch,
  type SetStateAction,
} from 'react';
import { useLocation, useNavigate } from 'react-router';
import { hasUnsavedDrafts } from './useAutosave';
import { io } from 'socket.io-client';
import { ApiError, request } from '../shared/lib/api';
import { isDesktop } from '../shared/lib/platform';
import { onBrowserResume } from './browserResume';
import type { Snapshot, OAuth, AuthStatus } from '../shared/lib/types';

export interface AccountSummary {
  id: string;
  instance: string | null;
  label: string;
  user_id: number | null;
  name: string;
  avatar_url: string | null;
  connected: boolean;
  unassigned: boolean;
  unavailable: boolean;
  paused: boolean;
  mining: NonNullable<Snapshot['mining']>['state'];
  game: string | null;
  reward: string | null;
  settings_revision: string;
}
export interface AccountLogin {
  id: string;
  target: string | null;
  code: OAuth | null;
  confirmed: boolean;
  user_id: number | null;
  name: string | null;
  avatar_url: string | null;
  error: string | null;
}
export interface AccountsOverview {
  instance: string;
  revision: number;
  accounts: AccountSummary[];
  login: AccountLogin | null;
}
export interface AccountTarget {
  id: string;
  revision: string;
}
export interface BulkResult {
  results: { id: string; ok: boolean; error?: string; revision?: string }[];
}
export const validAccountLabel = (value: string) =>
  new TextEncoder().encode(value).length <= 100 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
export const accountTarget = (account: AccountSummary): AccountTarget => ({
  id: account.id,
  revision: account.settings_revision,
});

interface ContextValue {
  overview: AccountsOverview | null;
  selected: string | null;
  connected: boolean;
  error: string;
  select: (id: string) => void;
  refresh: () => Promise<void>;
  action: <T = unknown>(action: object) => Promise<T>;
  views: Map<string, Map<string, unknown>>;
  restoreView: () => void;
}
const Context = createContext<ContextValue | null>(null);
const selectionKey = 'tdm.viewed-account';
function savedSelection() {
  try {
    return sessionStorage.getItem(selectionKey);
  } catch {
    return null;
  }
}
function rememberSelection(id: string) {
  try {
    sessionStorage.setItem(selectionKey, id);
  } catch {
    /* Optional browser storage. */
  }
}

export function AccountsProvider({ children }: { children: ReactNode }) {
  const [overview, setOverview] = useState<AccountsOverview | null>(null);
  const [defaultSelection, setSelected] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState('');
  const current = useRef<AccountsOverview | null>(null);
  const selection = useRef<string | null>(null);
  const views = useRef(new Map<string, Map<string, unknown>>());
  const routes = useRef(
    new Map<
      string,
      {
        pathname: string;
        search: string;
        hash: string;
        state: unknown;
        top: number;
        width: number;
        lists: [string, number, number][];
      }
    >(),
  );
  const restore = useRef(false);
  const location = useLocation();
  const requestedAccount = new URLSearchParams(location.search).get('account');
  // The committed route owns selection, including Back during an unfinished switch.
  const selected = requestedAccount
    ? overview?.accounts.some((account) => account.id === requestedAccount)
      ? requestedAccount
      : null
    : defaultSelection;
  selection.current = selected;
  const latestLocation = useRef(location);
  latestLocation.current = location;
  const navigate = useNavigate();
  useEffect(() => {
    if (!selected || !overview || overview.accounts.length <= 1) return;
    const params = new URLSearchParams(location.search);
    if (params.has('account')) return;
    params.set('account', selected);
    navigate(
      { pathname: location.pathname, search: `?${params}`, hash: location.hash },
      { replace: true, state: location.state },
    );
  }, [selected, overview?.accounts.length, location]);
  useEffect(() => {
    if (location.pathname === '/settings' && location.hash === '#notifications') return;
    const shared = views.current.get('shared');
    if ((shared?.get('telegram.action') as { value?: { busy: boolean } } | undefined)?.value?.busy)
      return;
    if (shared)
      for (const key of shared.keys()) if (key.startsWith('telegram.')) shared.delete(key);
  }, [location.pathname, location.hash]);
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (hasUnsavedDrafts()) event.preventDefault();
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, []);
  const receive = useCallback((value: AccountsOverview) => {
    if (!value || !Array.isArray(value.accounts)) return;
    if (current.current?.instance === value.instance && current.current.revision > value.revision)
      return;
    current.current = value;
    for (const id of views.current.keys())
      if (
        id !== 'shared' &&
        id !== 'none' &&
        !value.accounts.some((account) => account.id === id)
      ) {
        views.current.delete(id);
        routes.current.delete(id);
      }
    setOverview(value);
    setConnected(true);
    setError('');
    const requested = new URLSearchParams(latestLocation.current.search).get('account');
    const desired = requested ?? selection.current ?? savedSelection();
    const next =
      value.accounts.find((account) => account.id === desired)?.id ??
      (requested ? null : (value.accounts[0]?.id ?? null));
    selection.current = next;
    setSelected(next);
    if (next) rememberSelection(next);
    if (requested && !next) setError('account_not_found');
  }, []);
  const refresh = useCallback(async () => {
    receive(await request<AccountsOverview>('/api/accounts'));
  }, [receive]);
  useEffect(() => {
    const requested = new URLSearchParams(location.search).get('account');
    if (!requested || !current.current) return;
    if (current.current.accounts.some((account) => account.id === requested)) {
      selection.current = requested;
      setSelected(requested);
      rememberSelection(requested);
      setError('');
    } else {
      selection.current = null;
      setSelected(null);
      setError('account_not_found');
    }
  }, [location.search]);
  useEffect(() => {
    let disposed = false;
    const controller = new AbortController();
    if (isDesktop()) {
      void (async () => {
        while (!disposed) {
          try {
            const previous = current.current;
            const query = previous
              ? `?instance=${previous.instance}&revision=${previous.revision}`
              : '';
            const value = await request<AccountsOverview>(
              `/api/accounts${query}`,
              undefined,
              'GET',
              controller.signal,
            );
            if (!disposed) receive(value);
          } catch {
            if (disposed) break;
            setConnected(false);
            setError('request_failed');
            await new Promise<void>((resolve) => {
              const finish = () => {
                clearTimeout(timer);
                controller.signal.removeEventListener('abort', finish);
                resolve();
              };
              const timer = window.setTimeout(finish, 2000);
              controller.signal.addEventListener('abort', finish, { once: true });
            });
          }
        }
      })();
      return () => {
        disposed = true;
        controller.abort();
      };
    }
    const socket = io({ autoConnect: false, query: { protocol: '2', account: 'overview' } });
    let reconnectTimer: number | undefined;
    let failures = 0;
    const checkAuth = (reconnect = false) => {
      void request<AuthStatus>('/api/auth/status', undefined, 'GET', controller.signal)
        .then((auth) => {
          if (disposed) return;
          if (!auth.authenticated) window.dispatchEvent(new Event('auth-expired'));
          else if (reconnect && navigator.onLine) {
            clearTimeout(reconnectTimer);
            reconnectTimer = window.setTimeout(
              () => socket.connect(),
              Math.min(1000 * 2 ** Math.min(failures++, 4), 10_000),
            );
          }
        })
        .catch(() => {});
    };
    socket.on('accounts', (value) => {
      failures = 0;
      receive(value);
    });
    socket.on('disconnect', (reason) => {
      setConnected(false);
      if (reason !== 'io client disconnect') checkAuth(reason === 'io server disconnect');
    });
    socket.on('connect_error', () => {
      setConnected(false);
      setError('request_failed');
      checkAuth();
    });
    socket.on('notification', (value: { title: string; message: string }) => {
      if ('Notification' in window && Notification.permission === 'granted')
        new Notification(value.title, { body: value.message });
    });
    const offline = () => {
      socket.disconnect();
      setConnected(false);
    };
    const online = () => socket.connect();
    window.addEventListener('offline', offline);
    window.addEventListener('online', online);
    const stopResume = onBrowserResume(() => {
      if (disposed || !navigator.onLine) return;
      setConnected(false);
      socket.disconnect().connect();
    });
    socket.connect();
    void refresh().catch(() => {
      if (!disposed) setError('request_failed');
    });
    return () => {
      disposed = true;
      controller.abort();
      clearTimeout(reconnectTimer);
      stopResume();
      window.removeEventListener('offline', offline);
      window.removeEventListener('online', online);
      socket.removeAllListeners();
      socket.disconnect();
    };
  }, [receive, refresh]);

  const select = useCallback(
    (id: string) => {
      if (!current.current?.accounts.some((account) => account.id === id)) return;
      if (selection.current === id) return;
      if (selection.current)
        routes.current.set(selection.current, {
          ...latestLocation.current,
          top: pageScrollElement().scrollTop,
          width: window.innerWidth,
          lists: Array.from(document.querySelectorAll<HTMLElement>('[id]'))
            .filter((element) => element !== pageScrollElement() && element.scrollTop > 0)
            .map((element) => [element.id, element.scrollTop, element.clientWidth]),
        });
      const remembered = routes.current.get(id);
      const currentParams = new URLSearchParams(latestLocation.current.search);
      const rememberedParams = new URLSearchParams(remembered?.search);
      const samePage =
        remembered?.pathname === latestLocation.current.pathname &&
        remembered.hash === latestLocation.current.hash &&
        ['edit', 'tab', 'campaign', 'drop'].every(
          (key) => rememberedParams.get(key) === currentParams.get(key),
        );
      const sectionParams = new URLSearchParams();
      for (const key of ['edit', 'tab']) {
        const value = currentParams.get(key);
        if (value) sectionParams.set(key, value);
      }
      const destination = samePage
        ? remembered
        : {
            ...latestLocation.current,
            search: sectionParams.toString(),
            state: null,
          };
      const params = new URLSearchParams(destination.search);
      params.set('account', id);
      const state = {
        ...(typeof destination.state === 'object' ? destination.state : {}),
        preferencesFromSettings: latestLocation.current.state?.preferencesFromSettings === true,
      };
      restore.current = true;
      latestLocation.current = {
        ...latestLocation.current,
        ...destination,
        state,
        search: `?${params}`,
      };
      selection.current = id;
      setSelected(id);
      rememberSelection(id);
      setError('');
      navigate(
        { pathname: destination.pathname, search: params.toString(), hash: destination.hash },
        { state },
      );
    },
    [navigate],
  );
  const restoreView = useCallback(() => {
    if (!restore.current || !selection.current) return;
    const view = routes.current.get(selection.current);
    restore.current = false;
    document.getElementById('main')?.focus({ preventScroll: true });
    if (
      !view ||
      view.pathname !== latestLocation.current.pathname ||
      view.search !== latestLocation.current.search ||
      view.hash !== latestLocation.current.hash
    ) {
      pageScrollElement().scrollTo(0, 0);
      return;
    }
    for (const [id, top, width] of view.lists) {
      const element = document.getElementById(id);
      if (element?.clientWidth === width) element.scrollTop = top;
    }
    pageScrollElement().scrollTo(0, view.width === window.innerWidth ? view.top : 0);
  }, []);
  const action = useCallback(
    async <T,>(action: object): Promise<T> => {
      const result = await request<T>('/api/accounts', action);
      // A failed read after an accepted write must not report that write as failed.
      await refresh().catch(() => setError('request_failed'));
      return result;
    },
    [refresh],
  );
  return (
    <Context
      value={{
        overview,
        selected,
        connected,
        error,
        select,
        refresh,
        action,
        views: views.current,
        restoreView,
      }}
    >
      {children}
    </Context>
  );
}

export function useAccounts() {
  const value = useContext(Context);
  if (!value) throw new Error('Accounts provider missing');
  return value;
}

export function useAccountState<T>(
  key: string,
  initial: T | (() => T),
): [T, Dispatch<SetStateAction<T>>] {
  const { selected, views } = useAccounts();
  const id = selected ?? 'none';
  return useStoredState(views, id, key, initial);
}

export function useSharedState<T>(
  key: string,
  initial: T | (() => T),
): [T, Dispatch<SetStateAction<T>>] {
  return useStoredState(useAccounts().views, 'shared', key, initial);
}

function useStoredState<T>(
  views: Map<string, Map<string, unknown>>,
  id: string,
  key: string,
  initial: T | (() => T),
): [T, Dispatch<SetStateAction<T>>] {
  if (!views.has(id)) views.set(id, new Map());
  const saved = views.get(id)!;
  type Record = {
    value: T;
    listeners: Set<() => void>;
    subscribe: (listener: () => void) => () => void;
    snapshot: () => T;
    update: Dispatch<SetStateAction<T>>;
  };
  if (!saved.has(key)) {
    const record: Record = {
      value: typeof initial === 'function' ? (initial as () => T)() : initial,
      listeners: new Set(),
      subscribe(listener) {
        record.listeners.add(listener);
        return () => {
          record.listeners.delete(listener);
        };
      },
      snapshot() {
        return record.value;
      },
      update(next) {
        record.value =
          typeof next === 'function' ? (next as (previous: T) => T)(record.value) : next;
        for (const listener of record.listeners) listener();
      },
    };
    saved.set(key, record);
  }
  const record = saved.get(key) as Record;
  const value = useSyncExternalStore(record.subscribe, record.snapshot, record.snapshot);
  return [value, record.update];
}

export function accountRequest(id: string | null): typeof request {
  return (path, data, method, signal) => {
    if (!id || !/^(primary|[a-f0-9]{32})$/.test(id))
      return Promise.reject(new ApiError(409, 'account_required'));
    if (
      !/^\/api\/(settings(?:\?|$)|status(?:\?|$)|channels(?:\/|\?|$)|campaigns(?:\?|$)|console(?:\?|$)|games(?:\?|$)|history(?:\/|\?|$)|twitch\/logout$|oauth\/confirm$|reload$|cache\/clear$|mode\/exit-manual$)/.test(
        path,
      )
    )
      return Promise.reject(new ApiError(400, 'invalid_request'));
    return request(`/api/accounts/${id}${path.slice(4)}`, data, method, signal);
  };
}
