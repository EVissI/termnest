<script lang="ts">
  import { onMount } from 'svelte';
  import { TerminalView } from '../lib/terminal/TerminalView';
  import { sessionApi, type SessionId } from '../lib/ipc';

  let { sessionId = null }: { sessionId?: SessionId | null } = $props();

  let container: HTMLDivElement;
  let view = $state.raw<TerminalView | null>(null);

  export function size(): { cols: number; rows: number } {
    return { cols: view?.cols ?? 80, rows: view?.rows ?? 24 };
  }

  export function print(text: string): void {
    view?.write(text);
  }

  export function reset(): void {
    view?.reset();
  }

  export function focus(): void {
    view?.focus();
  }

  onMount(() => {
    const v = new TerminalView(container, {
      onInput: (data) => {
        if (sessionId !== null) sessionApi.write(sessionId, data).catch(() => {});
      },
      onResize: (cols, rows) => {
        if (sessionId !== null) sessionApi.resize(sessionId, cols, rows).catch(() => {});
      },
    });
    view = v;
    v.focus();
    return () => {
      view = null;
      v.dispose();
    };
  });

  // Subscribe to the output of the current session; replayed output arrives first.
  $effect(() => {
    const id = sessionId;
    const v = view;
    if (id === null || !v) return;

    let subscription: number | null = null;
    let cancelled = false;
    sessionApi
      .subscribe(id, (data) => v.write(data))
      .then((sub) => {
        if (cancelled) sessionApi.unsubscribe(id, sub).catch(() => {});
        else subscription = sub;
      })
      .catch(() => {});

    // The session may have been opened with a stale size; sync it once.
    sessionApi.resize(id, v.cols, v.rows).catch(() => {});

    return () => {
      cancelled = true;
      if (subscription !== null) sessionApi.unsubscribe(id, subscription).catch(() => {});
    };
  });
</script>

<div class="terminal" bind:this={container}></div>

<style>
  .terminal {
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
</style>
