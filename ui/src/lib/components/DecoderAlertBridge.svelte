<script lang="ts">
  import { onMount } from 'svelte';
  import { openEvents, type DecodedMessage, type ScannerEvent } from '$lib/api';
  import { loadDecoderAlertPrefs, maybeNotifyDecodedMessage } from '$lib/decoder-alerts';

  let ws: WebSocket | null = null;

  function handleEvent(event: ScannerEvent) {
    if (event.kind !== 'DecodedMessage') return;
    maybeNotifyDecodedMessage(event.data as DecodedMessage, loadDecoderAlertPrefs());
  }

  onMount(() => {
    ws = openEvents(handleEvent);
    return () => {
      ws?.close();
      ws = null;
    };
  });
</script>
