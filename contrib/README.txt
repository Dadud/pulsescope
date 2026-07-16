= PulseScope SDR server

The open-source SDR scanner serving a clean-room Web UI over HTTP+WS.

== Features ==
  * Native SoapySDR/RSP1B discovery, CF32 acquisition, retune/rate/reconnect
  * Bounded DSP: FM/WFM/AM/SSB demod, Goertzel tuning, CTCSS/DCS detection
  * Real native decoders: RDS, CW/Morse, DTMF
  * UDP streaming: PSAU audio + PSIQ complex IQ packets
  * CF32 file playback, IQ recording, per-bank overrides
  * Sidecar dependency manager: scan, install, run rtl_433/multimon-ng/direwolf/...
  * Bearer-token auth for server deployment

== Running standalone ==

  $ cargo build --release --features soapysdr
  $ PULSESCOPE_AUTH_TOKEN=my-secret ./target/release/pulsescope --server
    
  Browse to http://127.0.0.1:8765/ - include `Authorization: Bearer my-secret` for API calls.

== Config ==

Environment variables:

  PULSESCOPE_BIND         bind address           default 127.0.0.1
  PULSESCOPE_PORT         bind port              default 8765
  PULSESCOPE_AUTH_TOKEN   required for management endpoints; pass as Bearer
  PULSESCOPE_TLS_CERT     path to PEM server cert (or chain)
  PULSESCOPE_TLS_KEY      path to PEM private key
  PULSESCOPE_UI_DIR       override location of the SvelteKit `build/` bundle
  PULSESCOPE_SOAPY_UTIL   override binary path of SoapySDRUtil
  PULSESCOPE_DATA_DIR     override where recordings/devisions live (default ~/pulsescope)

When both PULSESCOPE_TLS_CERT and PULSESCOPE_TLS_KEY are set, the server speaks
HTTPS on the same port using rustls. All endpoints (including `/health` and
static UI) are served over the same TLS. Both `/api/*` and bare `/...` paths
work over HTTP and HTTPS.

For Linux without SoapySDR installed system-wide, install SoapySDR via apt: `apt-get install libsoapysdr-dev soapysdr-tools`

== Docker ==

  $ docker build -t pulsescope .
  $ docker run -d --name pulsescope --restart=unless-stopped \
      -p 8765:8765 \
      -e PULSESCOPE_AUTH_TOKEN=my-secret \
      -v pulsescope-data:/var/lib/pulsescope \
      pulsescope
  $ docker logs -f pulsescope

The image runs the binary with `--server`, exposes port 8765, and persists data
to a Docker volume. Hardware (RTL-SDR/HackRF/RSP1B/Airspy) must be passed through
with `--device=/dev/bus/usb/...` or similar.

== systemd (Linux) ==

  $ sudo cp contrib/systemd/pulsescope.service /etc/systemd/system/
  $ sudo install -d -opulsescope -gpulsescope /var/lib/pulsescope
  $ sudo install -m 600 /dev/null /etc/pulsescope/pulsescope.env
  $ printf 'PULSESCOPE_AUTH_TOKEN=my-secret\n' | sudo tee -a /etc/pulsescope/pulsescope.env
  $ sudo systemctl daemon-reload
  $ sudo systemctl enable --now pulsescope

== CLI options ==

  pulsescope         -> launches the Tauri desktop shell (default)
  pulsescope --server   -> launches the standalone server (no UI dependency)

== Linux dependency provisioning ==

  apt-get install libsoapysdr-dev soapysdr-tools rtl-sdr librtlsdr-dev
  apt-get install rtl-433 multimon-ng direwolf
  # AIS-catcher and dumpvdl2 etc. ship pre-built tarballs; see /decoders/install/:name

== macOS ==

  brew install soapysdr rtl-433 multimon-ng
  ./target/release/pulsescope --server
