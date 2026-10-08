import { createContext, useContext, useEffect, useReducer, useRef, type ReactNode } from 'react';
import type { Snapshot } from '../shared/lib/types';
import { connectBrowser, type Subscription } from './transport';
import { I18n } from '../shared/lib/i18n';
import { useAutosave } from './useAutosave';
import { initialState, reducer } from './reducer';

export function upsert<T extends { id: string | number }>(items: T[], item: T): T[] {
  return items.some((current) => current.id === item.id)
    ? items.map((current) => (current.id === item.id ? item : current))
    : [...items, item];
}
const Context = createContext<{
  data: Snapshot | null;
  connected: boolean;
  incompatible: boolean;
  historyRevision: number;
  autosave: ReturnType<typeof useAutosave>;
}>({
  data: null,
  connected: false,
  incompatible: false,
  historyRevision: 0,
  autosave: null as unknown as ReturnType<typeof useAutosave>,
});

export function MinerProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);
  const transport = useRef<Subscription | null>(null);
  const autosave = useAutosave(state.data?.settings, state.hydrated, (settings, revision) => {
    dispatch({ type: 'settings', settings, revision });
  });
  useEffect(() => {
    if (state.resync) transport.current?.resync();
  }, [state.resync]);
  useEffect(() => {
    const subscription = connectBrowser(dispatch);
    transport.current = subscription;
    return () => {
      transport.current = null;
      subscription.close();
    };
  }, []);
  return (
    <Context
      value={{
        data: state.data,
        connected: state.hydrated,
        incompatible: state.incompatible,
        historyRevision: state.data?.history_clear_revision ?? 0,
        autosave,
      }}
    >
      <I18n>{children}</I18n>
    </Context>
  );
}
export const useMiner = () => useContext(Context);
