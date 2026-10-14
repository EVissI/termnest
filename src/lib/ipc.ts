import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type SessionId = number;

export type SessionState = 'connecting' | 'connected' | 'reconnecting' | 'disconnected' | 'error';

/** Error object returned by the Rust core (`CoreError`). */
export interface CoreError {
  kind: string;
  message: string;
}

export interface SessionStateEvent {
  id: SessionId;
  state: SessionState;
  error: CoreError | null;
}

export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') {
    return e.message;
  }
  return String(e);
}

export const sessionApi = {
  open(target: string, keyPath: string, cols: number, rows: number): Promise<SessionId> {
    return invoke('session_open', { target, keyPath, cols, rows });
  },

  /** Subscribes to session output. Buffered output is replayed first. Returns the subscription id. */
  subscribe(id: SessionId, onData: (data: Uint8Array) => void): Promise<number> {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buffer) => onData(new Uint8Array(buffer));
    return invoke('session_subscribe', { id, onOutput: channel });
  },

  unsubscribe(id: SessionId, subscription: number): Promise<void> {
    return invoke('session_unsubscribe', { id, subscription });
  },

  write(id: SessionId, data: Uint8Array): Promise<void> {
    return invoke('session_write', { id, data: Array.from(data) });
  },

  resize(id: SessionId, cols: number, rows: number): Promise<void> {
    return invoke('session_resize', { id, cols, rows });
  },

  close(id: SessionId): Promise<void> {
    return invoke('session_close', { id });
  },

  defaultKeyPath(): Promise<string | null> {
    return invoke('default_key_path');
  },

  onState(handler: (event: SessionStateEvent) => void): Promise<UnlistenFn> {
    return listen<SessionStateEvent>('session://state', (e) => handler(e.payload));
  },
};
