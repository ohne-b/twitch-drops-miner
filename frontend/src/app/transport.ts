import { io, type Socket } from 'socket.io-client';
import type { AuthStatus, ServerEvents } from '../shared/lib/types';
import { request } from '../shared/lib/api';
import type { StateAction } from './reducer';

export interface Subscription {
  resync(): void;
  close(): void;
}

export function connectBrowser(receive: (action: StateAction) => void): Subscription {
  const socket: Socket<ServerEvents, { state_resync: () => void }> = io({
    autoConnect: false,
    query: { protocol: '2' },
  });
  let disposed = false;
  let incompatible = false;
  const checkAuth = (reconnect = false) => {
    void request<AuthStatus>('/api/auth/status')
      .then((auth) => {
        if (disposed) return;
        if (!auth.authenticated) window.dispatchEvent(new Event('auth-expired'));
        else {
          window.dispatchEvent(new Event('auth-updated'));
          if (reconnect && !incompatible) socket.connect();
        }
      })
      .catch(() => {});
  };
  const mismatch = () => {
    incompatible = true;
    receive({ type: 'incompatible' });
  };
  socket.on('state_snapshot', (value) => receive({ type: 'snapshot', value }));
  socket.on('state_patch', (value) => receive({ type: 'patch', value }));
  socket.on('protocol_mismatch', mismatch);
  socket.on('initial_state', mismatch);
  socket.on('disconnect', (reason) => {
    receive({ type: 'disconnect' });
    checkAuth(reason === 'io server disconnect');
  });
  socket.on('connect_error', () => {
    receive({ type: 'disconnect' });
    checkAuth();
  });
  socket.on('notification', (value) => {
    if ('Notification' in window && Notification.permission === 'granted')
      new Notification(value.title, { body: value.message });
  });
  socket.connect();
  return {
    resync: () => socket.emit('state_resync'),
    close: () => {
      disposed = true;
      socket.removeAllListeners();
      socket.disconnect();
    },
  };
}
