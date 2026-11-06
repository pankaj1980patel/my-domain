# my-domain · server (registry)

An authenticated **device registry** for the [my-domain](https://github.com/pankaj1980patel/my-domain)
messenger. Users log in, register their devices, and look up their **other** devices' endpoints
(IP + TCP/UDP ports) — so clients connect directly without relying on LAN discovery.

- `POST /auth/register` / `POST /auth/login` → JWT.
- A device `POST /devices/register`s its identity on login and on **network change** (no polling).
- Peers `GET /devices` to obtain the user's other devices.
- Messaging stays peer-to-peer and **end-to-end encrypted** — the server only stores endpoints, never
  message content or encryption keys.

Backed by **MongoDB**. Config comes from `.env` (never committed).

## Run

```bash
cp .env.example .env     # fill in MONGO_URI, MONGO_DB, JWT_SECRET
cargo run                # listens on http://0.0.0.0:8080 (PORT to override)
```

## API

| Method | Path | Auth | Body / Query |
|--------|------|------|--------------|
| POST | `/auth/register` | — | `{username,password}` → `{token,username}` |
| POST | `/auth/login` | — | `{username,password}` → `{token,username}` |
| POST | `/auth/verify` | — | `{username,password}` → 200/401 (confirm password for a settings change) |
| POST | `/devices/register` | JWT | `{node_id,name,ip?,tcp_port,udp_port}` — upsert (also the network-change update) |
| GET  | `/devices` | JWT | `?exclude=<node_id>` → this user's devices |
| POST | `/devices/unregister` | JWT | `{node_id}` |
| GET  | `/health` | — | liveness + Mongo ping |

```bash
TOKEN=$(curl -s -X POST localhost:8080/auth/register -H 'content-type: application/json' \
  -d '{"username":"alice","password":"correct horse"}' | jq -r .token)
curl -X POST localhost:8080/devices/register -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"node_id":"abc","name":"laptop","ip":"192.168.0.10","tcp_port":51011,"udp_port":60539}'
curl localhost:8080/devices -H "authorization: Bearer $TOKEN"
```

## Config (`.env`)

| Var | Purpose |
|-----|---------|
| `MONGO_URI` | MongoDB connection string (Atlas SRV URI) |
| `MONGO_DB`  | Database name (default `device_registry`) |
| `JWT_SECRET`| HS256 signing secret (`openssl rand -base64 48`) |
| `PORT`      | HTTP port (default 8080) |

## Limits

Abuse limits live in memory (a restart resets them; every map drops idle entries and holds at most
100k keys). Over a rate limit the answer is `429` with `Retry-After` (seconds); an oversized body is
`413`, an oversized field `400` naming it. Defaults suit a handful of devices per account; raise them in
`.env` (positive numbers only, anything else keeps the default) and restart.

| Var | Default | What |
|-----|---------|------|
| `MAX_DEVICES_PER_ACCOUNT` | 32 | Devices one account may register (a new one past it gets `409`) |
| `SIGNALS_PER_MIN` | 600 | `/signal` + `/signal/selfping` requests per account a minute (bursts of a fifth) |
| `PUSH_PER_MIN` | 60 | FCM pushes per account a minute (bursts of half) |
| `PUSH_PER_TARGET_PER_MIN` | 30 | FCM pushes to one device a minute (bursts of a third) |
| `PUSH_PER_DAY` | 5000 | FCM pushes per account a day |
| `PUSH_MAX_BYTES` | 3584 | Largest push payload (FCM's own cap is 4 KB) |
| `PUSH_DEDUPE_SECS` | 10 | The same push again within this is answered ok but not sent |
| `PAIR_FAILS_PER_CLIENT` | 10 | Wrong pairing claims per address (IPv6 /64) a minute before a lockout of 1 min, doubling per lockout up to 1 h |
| `PAIR_FAILS_GLOBAL` | 120 | Wrong codes a minute server-wide past which each address's allowance halves (soft; nobody is blocked by it) |
| `PAIR_CODE_LOCK_AFTER` | 1000 | Wrong codes tried while a pairing is live before its 6-digit code stops working (the QR still works) |
| `RELAY_BYTES_PER_SEC` | 4 MiB | Relay bandwidth per account (senders over it are slowed, not dropped) |

Fixed caps: request bodies 4 KiB on `/auth/*`, 8 KiB on `/pair/*`, 16 KiB on `/devices/*`, 32 KiB on
`/signal` (its `data` at most 16 KiB); device fields as in `devices.rs` (`MAX_*`). `JWT_SECRET` should be
at least 32 bytes (a shorter one is warned about at startup). See `docs/SECURITY-REVIEW-2026-11.md`
(M18, M20, M21, L24).

## Notes

- Collections are namespaced **`md_users`** / **`md_devices`** so this server never collides with other
  apps sharing the same database.
- Devices are **per-user**: you only ever see/update your own devices.
- No heartbeat TTL — entries persist until overwritten or unregistered (clients don't poll).
- Passwords are Argon2id-hashed; the **encryption key** for E2EE lives only on the clients and is never
  sent here. In production the client↔server hop should be HTTPS (the Atlas hop already is TLS).
