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

self.addEventListener('fetch', (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin || url.pathname.startsWith('/api/') || url.pathname === '/spectrum' || url.pathname.startsWith('/audio/')) return;

  if (request.mode === 'navigate') {
    // Receiver shells must update immediately after deployment. Fall back to
    // the cached shell only when the LAN appliance is genuinely unreachable.
    event.respondWith(fetch(request, { cache: 'no-store' }).catch(() => caches.match('/index.html').then((response) => response ?? Response.error())));
    return;
  }

  event.respondWith(caches.match(request).then((cached) => cached ?? fetch(request)));
});
