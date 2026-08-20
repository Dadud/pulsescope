/// <reference lib="webworker" />

import { build, files, version } from '$service-worker';

declare const self: ServiceWorkerGlobalScope;

const cacheName = `pulsescope-ui-${version}`;
const shell = [...build, ...files];

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(cacheName).then((cache) => cache.addAll(shell)));
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((key) => key.startsWith('pulsescope-ui-') && key !== cacheName).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('message', (event) => {
  const data = event.data as { type?: string; title?: string; body?: string; tag?: string; hash?: string };
  if (data?.type !== 'decoder-alert' || !data.title) return;
  event.waitUntil(
  self.registration.showNotification(data.title, {
    body: data.body ?? '',
    tag: data.tag ?? undefined,
    data: { hash: data.hash ?? '#/messages' },
  }),
  );
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const hash = (event.notification.data?.hash as string | undefined) ?? '#/messages';
  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((clients) => {
      for (const client of clients) {
        if ('focus' in client) {
          const url = new URL(client.url);
          url.hash = hash;
          return client.focus().then(() => client.navigate(url.toString()));
        }
      }
      return self.clients.openWindow(`${self.location.origin}/${hash.startsWith('#') ? hash : `#${hash}`}`);
    }),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin || url.pathname.startsWith('/api/') || url.pathname === '/spectrum' || url.pathname.startsWith('/audio/')) return;

  if (request.mode === 'navigate') {
    event.respondWith(fetch(request, { cache: 'no-store' }).catch(() => caches.match('/index.html').then((response) => response ?? Response.error())));
    return;
  }

  event.respondWith(caches.match(request).then((cached) => cached ?? fetch(request)));
});
