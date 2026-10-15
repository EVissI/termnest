<script lang="ts" module>
  // Simple 24x24 stroke icons (drawn for termnest, no external icon font).
  const paths: Record<string, string> = {
    plus: 'M12 5v14M5 12h14',
    x: 'M6 6l12 12M18 6L6 18',
    server: 'M4 4h16v6H4zM4 14h16v6H4zM8 7h.01M8 17h.01',
    terminal: 'M4 5h16v14H4zM7 9l3 3-3 3M12 15h5',
    key: 'M15 7a4 4 0 1 1-3.9 4.8L4 19v-3h3v-2h2l2.2-2.2A4 4 0 0 1 15 7zM16 9h.01',
    code: 'M8 8l-4 4 4 4M16 8l4 4-4 4M14 5l-4 14',
    settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1',
    folder: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
    'folder-plus': 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM12 10v6M9 13h6',
    file: 'M6 3h8l4 4v14H6zM14 3v4h4',
    link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
    upload: 'M12 16V4M7 9l5-5 5 5M4 20h16',
    download: 'M12 4v12M7 11l5 5 5-5M4 20h16',
    refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7',
    up: 'M12 19V5M6 11l6-6 6 6',
    home: 'M4 11l8-7 8 7v9H4zM10 20v-6h4v6',
    search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4',
    copy: 'M9 9h11v11H9zM5 15V4h11',
    edit: 'M4 20h4L19 9l-4-4L4 16zM14 6l4 4',
    trash: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13',
    'split-h': 'M4 4h16v16H4zM12 4v16',
    'split-v': 'M4 4h16v16H4zM4 12h16',
    broadcast: 'M12 12h.01M8.5 15.5a5 5 0 0 1 0-7M15.5 8.5a5 5 0 0 1 0 7M5.6 18.4a9 9 0 0 1 0-12.8M18.4 5.6a9 9 0 0 1 0 12.8',
    'chevron-right': 'M9 6l6 6-6 6',
    'chevron-down': 'M6 9l6 6 6-6',
    'chevron-left': 'M15 6l-6 6 6 6',
    lock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3',
    warning: 'M12 3l10 18H2zM12 10v4M12 17h.01',
    check: 'M5 12l5 5 9-10',
    more: 'M5 12h.01M12 12h.01M19 12h.01',
    window: 'M3 5h18v14H3zM3 9h18',
    import: 'M12 3v12M7 10l5 5 5-5M5 21h14',
    play: 'M7 4l12 8-12 8z',
    sidebar: 'M4 4h16v16H4zM9 4v16',
    stop: 'M6 6h12v12H6z',
    duplicate: 'M8 8h12v12H8zM4 16V4h12',
    agent: 'M12 3l1.8 4.2L18 9l-4.2 1.8L12 15l-1.8-4.2L6 9l4.2-1.8zM18 15l.9 2.1L21 18l-2.1.9L18 21l-.9-2.1L15 18l2.1-.9zM5 15l.6 1.4L7 17l-1.4.6L5 19l-.6-1.4L3 17l1.4-.6z',
  };

  export type IconName = keyof typeof paths;
</script>

<script lang="ts">
  let { name, size = 16, title }: { name: string; size?: number; title?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden={title ? undefined : 'true'}
  role={title ? 'img' : undefined}
>
  {#if title}<title>{title}</title>{/if}
  <path d={paths[name] ?? ''} />
</svg>
