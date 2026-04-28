# blazar

*Rate-limited Rust mail relay backing a contact-form endpoint.*

Named after the blazar — an active galactic nucleus with relativistic jets
pointed at Earth, emitting in powerful high-energy bursts. Fits the
rate-limited burst-sending pattern of a contact-form backend with a hard
daily cap.

## Stack

| Layer        | Choice                                                      |
|--------------|-------------------------------------------------------------|
| Language     | Rust (stable, edition 2021)                                 |
| HTTP         | `axum` 0.8                                                  |
| SMTP client  | `lettre` 0.11 (STARTTLS, SASL PLAIN)                        |
| Rate limit   | `tower_governor` (per-IP, in-memory)                        |
| Container    | distroless/static                                           |
| Binary       | musl static, stripped                                       |

## Architecture (high level)

```
Browser
    │
    ▼
CDN (TLS terminates here)
    │
    ▼
Origin (host nginx, TLS)
    │   proxy_pass -> 127.0.0.1:<port>
    ▼
Blazar (Rust / axum)
    GET  /health  → 200 OK
    GET  /nonce   → HMAC-signed nonce (short TTL)
    POST /contact → defense chain → SMTP send  OR  disk queue
    │
    ▼
SMTP (STARTTLS, SASL PLAIN)
```

## Abuse defenses (layered)

Every request traverses the stack in order; any failure short-circuits
without revealing which layer rejected (silent reject = `204 No Content`).

1. **CORS strict** — single allowed origin; pre-flight rejected for
   everything else.
2. **Honeypot field** — if the hidden input is non-empty, respond `204` as
   if successful. Bots that blindly fill every field get filtered out
   without feedback.
3. **HMAC nonce** — `/nonce` issues a short-TTL HMAC-SHA256 signature over
   `(timestamp, random)`; `/contact` re-verifies with constant-time
   compare. Expired or forged nonces silent-reject.
4. **Per-IP rate limit** — `tower_governor` token-bucket per source IP.
   Tuned so an honest user retrying a few times is not punished, while
   sustained abuse is capped.
5. **Global daily cap** — on-disk counter caps total sends per UTC day.
   Over cap, the submission is enqueued to disk instead of sent — the disk
   queue is flushed by a background task at the daily reset.
6. **Silent reject** — `204 No Content` is returned on every abuse path
   so an attacker cannot distinguish a rejected submission from a
   successful one.

## Routes

| Method | Path       | Purpose                                              |
|--------|------------|------------------------------------------------------|
| GET    | `/health`  | Liveness probe — returns `200 OK`                   |
| GET    | `/nonce`   | Issues an HMAC-signed nonce with a short TTL         |
| POST   | `/contact` | Contact-form submission; full defense chain runs     |

## Local dev

```bash
cp .env.example .env
# fill in dummy SMTP_PASS + a 64-hex-char NONCE_SECRET
cargo check          # fast type-check
cargo test           # run unit tests
cargo run            # bind on 127.0.0.1:3030
```

Production image build (identical to CI):

```bash
docker build -t blazar:dev .
docker run --rm -p 3030:3030 --env-file .env blazar:dev
```

## Image

- Base: distroless/static — nothing but the binary + CA certs; no shell,
  no package manager.
- Binary: statically linked via musl, stripped.
- User: non-root.
- No writable filesystem inside the container; the queue dir mounts as a
  named docker volume.

## Status

Personal infrastructure project. Not accepting external contributions.
