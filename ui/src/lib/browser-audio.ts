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
    this.nextPlayTime = 0;
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

    const framesPerChannel = Math.floor(frame.samples.length / frame.channels);
    const buffer = context.createBuffer(frame.channels, framesPerChannel, frame.sampleRate);
    for (let channel = 0; channel < frame.channels; channel += 1) {
      const output = buffer.getChannelData(channel);
      for (let i = 0; i < framesPerChannel; i += 1) output[i] = frame.samples[i * frame.channels + channel];
    }

    const minimumStart = context.currentTime + 0.18;
    if (this.nextPlayTime < context.currentTime + 0.03) {
      if (this.nextPlayTime > 0) this.underruns += 1;
      this.nextPlayTime = minimumStart;
    } else if (this.nextPlayTime > context.currentTime + 0.55) {
      this.nextPlayTime = minimumStart;
    }
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    source.start(this.nextPlayTime);
    this.nextPlayTime += buffer.duration;
    this.stateListener('playing');
  }
}
