import { isDesktop } from './platform';
export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
export async function request<T>(
  path: string,
  data?: unknown,
  method = data === undefined ? 'GET' : 'POST',
  signal?: AbortSignal,
): Promise<T> {
  if (isDesktop()) {
    const { desktopRequest } = await import('./desktop');
    return desktopRequest<T>(path, data, method, signal);
  }
  const response = await fetch(path, {
    method,
    credentials: 'same-origin',
    signal,
    headers: {
      'Content-Type': 'application/json',
      ...(method === 'GET' ? {} : { 'X-TDM-Request': '1' }),
    },
    body: data === undefined ? undefined : JSON.stringify(data),
  });
  if (!response.ok) {
    const body: unknown = await response.json().catch(() => null);
    const detail =
      body && typeof body === 'object' && 'detail' in body && typeof body.detail === 'string'
        ? body.detail
        : 'request_failed';
    if (response.status === 401 && detail === 'authentication_required')
      window.dispatchEvent(new Event('auth-expired'));
    throw new ApiError(response.status, detail);
  }
  return response.json() as Promise<T>;
}
export function safeUrl(value: string | undefined | null): string | undefined {
  if (!value) return undefined;
  try {
    const url = new URL(value);
    return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}
export function moveGame(games: string[], from: number, to: number): string[] {
  if (!Number.isInteger(to) || !Number.isInteger(from) || from < 0 || from >= games.length)
    return games;
  const next = [...games];
  const [game] = next.splice(from, 1);
  if (game !== undefined) next.splice(Math.max(0, Math.min(next.length, to)), 0, game);
  return next;
}
