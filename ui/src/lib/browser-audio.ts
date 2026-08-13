import { openAudio, type PcmAudioFrame } from '$lib/api';

export type BrowserAudioState = 'off' | 'connecting' | 'buffering' | 'playing' | 'reconnecting' | 'error';

/**
 * Compatibility browser PCM player. It uses scheduled native Web Audio buffers
 * rather than main-thread DSP. WebRTC/Opus will replace the transport, while
 * this remains the observable fallback for LAN browsers without media support.
 */
export class BrowserAudio {
  private context: AudioContext | null = null;
  private socket: WebSocket | null = null;
  private nextPlayTime = 0;
  private expectedSequence: number | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private wanted = false;
  private stateListener: (state: BrowserAudioState) => void;
  private pendingFrames: PcmAudioFrame[] = [];
  private pendingSamples = 0;
  private pendingFormat = '';
  private scheduledSources = new Set<AudioBufferSourceNode>();

  lostFrames = 0;
  underruns = 0;

  constructor(onState: (state: BrowserAudioState) => void) {
    this.stateListener = onState;
  }

  async start(): Promise<void> {
    this.wanted = true;
    if (!this.context) this.context = new AudioContext({ latencyHint: 'playback', sampleRate: 48_000 });
    await this.context.resume();
    if (!this.socket || this.socket.readyState > WebSocket.OPEN) this.connect();
  }

  stop(): void {
    this.wanted = false;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.socket?.close();
    this.socket = null;
    this.expectedSequence = null;
    this.resetPlayback(true);
    void this.context?.suspend();
    this.stateListener('off');
  }

  private connect(): void {
    if (!this.wanted) return;
    this.stateListener(this.socket ? 'reconnecting' : 'connecting');
    this.socket = openAudio(
      (frame) => this.schedule(frame),
      (state) => {
        if (state === 'open') this.stateListener('buffering');
        if ((state === 'closed' || state === 'error') && this.wanted) this.reconnect();
      },
    );
  }

  private reconnect(): void {
    if (this.reconnectTimer || !this.wanted) return;
    this.stateListener('reconnecting');
    this.resetPlayback(false);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, 1_000);
  }

  private schedule(frame: PcmAudioFrame): void {
    const context = this.context;
    if (!context || context.state !== 'running') return;
    if (this.expectedSequence !== null && frame.sequence > this.expectedSequence) {
      this.lostFrames += frame.sequence - this.expectedSequence;
    }
    this.expectedSequence = frame.sequence + 1;

    const format = `${frame.sampleRate}/${frame.channels}`;
    if (this.pendingFormat && this.pendingFormat !== format) this.resetPlayback(true);
    this.pendingFormat = format;
    this.pendingFrames.push(frame);
    this.pendingSamples += frame.samples.length;
    // The PCM compatibility path batches three 20 ms wire frames. Creating a
    // Web Audio source fifty times per second caused avoidable main-thread and
    // garbage-collector pressure on phones; 60 ms batches preserve the LAN
    // jitter target while reducing scheduling churn by two thirds.
    const batchSamples = Math.round(frame.sampleRate * frame.channels * 0.06);
    if (this.pendingSamples < batchSamples) return;

    let sampleCount = 0;
    let frameCount = 0;
    while (frameCount < this.pendingFrames.length && sampleCount < batchSamples) {
      sampleCount += this.pendingFrames[frameCount].samples.length;
      frameCount += 1;
    }
    const frames = this.pendingFrames.splice(0, frameCount);
    this.pendingSamples -= sampleCount;
    const samples = new Float32Array(sampleCount);
    let offset = 0;
    for (const pending of frames) {
      samples.set(pending.samples, offset);
      offset += pending.samples.length;
    }

    const framesPerChannel = Math.floor(samples.length / frame.channels);
    const buffer = context.createBuffer(frame.channels, framesPerChannel, frame.sampleRate);
    for (let channel = 0; channel < frame.channels; channel += 1) {
      const output = buffer.getChannelData(channel);
      for (let i = 0; i < framesPerChannel; i += 1) output[i] = samples[i * frame.channels + channel];
    }

    const minimumStart = context.currentTime + 0.18;
    if (this.nextPlayTime < context.currentTime + 0.03) {
      if (this.nextPlayTime > 0) this.underruns += 1;
      this.nextPlayTime = minimumStart;
    } else if (this.nextPlayTime > context.currentTime + 0.55) {
      this.resetPlayback(true);
      this.nextPlayTime = minimumStart;
    }
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    this.scheduledSources.add(source);
    source.onended = () => this.scheduledSources.delete(source);
    source.start(this.nextPlayTime);
    this.nextPlayTime += buffer.duration;
    this.stateListener('playing');
  }

  private resetPlayback(stopScheduled: boolean): void {
    this.pendingFrames = [];
    this.pendingSamples = 0;
    this.pendingFormat = '';
    this.nextPlayTime = 0;
    if (!stopScheduled) return;
    for (const source of this.scheduledSources) {
      try { source.stop(); } catch { /* already ended */ }
    }
    this.scheduledSources.clear();
  }
}
