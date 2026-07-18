# PulseScope security and privacy

## Supported deployment threat model

### Local desktop (supported, default)

The API binds to `127.0.0.1`. PulseScope assumes the OS account and desktop
session are trusted, but treats browser content and other local processes as
potentially hostile. Keep the loopback binding, filesystem permissions, and
Tauri sandbox intact. A local attacker running as the same OS user, a malicious
decoder binary, compromised SDR firmware, and physical attacks are out of
scope. The desktop API has no token by default because loopback is part of this
single-user trust boundary.

### Trusted LAN (supported with controls)

Set an explicit private `PULSESCOPE_BIND`, TLS certificate/key, a randomly
generated `PULSESCOPE_AUTH_TOKEN`, and exact comma-separated
`PULSESCOPE_CORS_ORIGINS`. LAN clients and infrastructure administrators are
trusted; other devices on the network are not. TLS is required whenever a
credential or received message crosses the host. Never expose decoder streams,
the API, or the UI directly through router port forwarding.

Generate at least 32 random bytes (for example `openssl rand -base64 32`) and
provide the value through a secret manager or protected environment file, not
shell history, URLs, source control, or command-line arguments. An authenticated
administrator can atomically rotate it with `POST /auth/token/rotate` or revoke
all access with `POST /auth/token/revoke`; persist a rotated value in the secret
manager before restarting. Authentication decisions use
constant-time comparison and security events record action/outcome without the
credential. WebSockets use 30-second, single-use bootstrap tickets.

### Internet-facing and multi-user (supported only behind an identity gateway)

The built-in token is a single-user/operator credential, not a multi-user
identity system. Internet-facing operation **must** put PulseScope on a private
loopback or service network behind a maintained identity-aware reverse proxy.
The gateway must provide individual identities via password authentication
(Argon2id/scrypt/bcrypt plus MFA) or OIDC/SAML, short-lived Secure/HttpOnly/
SameSite sessions, logout and session revocation, and `viewer`, `operator`, and
`admin` roles. It must enforce authorization per route: viewers may read,
operators may tune/scan/record, and only admins may change settings, install
decoders, export diagnostics, or manage users. State-changing cookie requests
must use an unpredictable CSRF token and Origin validation. The gateway passes
the PulseScope service token only after authorization; never expose that token
to browsers. Direct public binding is unsupported.

## Transport and resource controls

The server limits request bodies to 1 MiB, requests to 30 seconds, concurrent
requests to 64, total traffic to 200 requests/second, and external lookup paths
to 10 requests/second. Deployments should add per-identity limits at their
gateway. CORS is an exact allowlist and wildcard origins are ignored. Responses
set no-store, CSP, frame denial, no-sniff, and no-referrer headers.

Treat device strings and decoder arguments as untrusted. Only select devices,
decoders, and controls returned by discovery; never interpolate values into a
shell. Import/export paths must remain under the configured data directory,
filenames must be a single normal component, and URLs must be HTTPS with an
explicit allowlisted host. Configuration should be schema-validated and use
bounded numeric values. The API rejects oversized input globally; new handlers
must add domain validation before invoking a device, decoder, filesystem, or
network operation.

## Privacy controls

External aircraft/radio lookups, online maps/tiles, crash reporting,
cloud transcription, and decoder downloads are opt-in features. Before enabling
one, disclose the destination, fields transmitted, retention policy, and
whether coordinates, identifiers, audio, or message content leave the host.
Offer an offline/no-network mode and locally hosted map/transcription options.
Pin decoder download hosts and verify signatures or published hashes.

Logs and diagnostics must never contain Authorization/Cookie values, URL query
strings, session/ticket values, precise coordinates, personal identifiers, raw
audio, or decoded message bodies. Record an opaque request/correlation ID,
coarse operation, actor role, outcome, and timestamp instead. Diagnostics
exports require admin authorization, preview, explicit consent, and the same
redaction pass; security audit records should be append-only with restricted
access and a documented retention period.

## Reporting

Do not open a public issue containing a vulnerability or received radio data.
Use the repository owner's private security-reporting channel. Include version,
deployment mode, reproduction steps, and impact, with all secrets and personal
data removed.
