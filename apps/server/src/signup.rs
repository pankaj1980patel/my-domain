//! Sign-up limits: who may make an account, and how fast.
//!
//! - `REGISTRATION`: `open` (default), `invite` (an invite code from
//!   `POST /admin/invites` is needed; codes are stored only as a hash) or
//!   `closed` (403).
//! - Per client address (an IPv6 /64 counts as one): `REGISTER_PER_MIN_PER_CLIENT`
//!   tries a minute and `REGISTER_PER_DAY_PER_CLIENT` accounts a day. The
//!   daily count is kept in the store, so a restart doesn't reset it.
//! - Server-wide: `REGISTER_PER_MIN` tries a minute.
//!
//! Over a limit: 429 with `Retry-After`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use rand_core::{OsRng, RngCore};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::limits::{env_or, Buckets, StoredWindows};
use crate::store::Store;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Open,
    Invite,
    Closed,
}

impl Mode {
    /// `REGISTRATION` as given; anything unknown is `closed` (a typo must
    /// not open sign-ups that were meant to be shut).
    pub fn parse(v: Option<&str>) -> Mode {
        match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("open") => Mode::Open,
            Some("invite") => Mode::Invite,
            Some(other) => {
                if other != "closed" {
                    tracing::warn!("REGISTRATION={other:?} is not open|invite|closed; closed");
                }
                Mode::Closed
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Open => "open",
            Mode::Invite => "invite",
            Mode::Closed => "closed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SignupConfig {
    pub mode: Mode,
    pub per_client_per_min: f64,
    pub per_min: f64,
    pub per_client_per_day: u32,
}

impl Default for SignupConfig {
    fn default() -> Self {
        SignupConfig {
            mode: Mode::Open,
            per_client_per_min: 3.0,
            per_min: 30.0,
            per_client_per_day: 5,
        }
    }
}

impl SignupConfig {
    pub fn from_env() -> Self {
        let d = Self::default();
        SignupConfig {
            mode: Mode::parse(std::env::var("REGISTRATION").ok().as_deref()),
            per_client_per_min: env_or("REGISTER_PER_MIN_PER_CLIENT", d.per_client_per_min),
            per_min: env_or("REGISTER_PER_MIN", d.per_min),
            per_client_per_day: env_or("REGISTER_PER_DAY_PER_CLIENT", d.per_client_per_day),
        }
    }
}

const DAY: Duration = Duration::from_secs(24 * 3600);
const MAX_TRACKED: usize = 100_000;

struct Rate {
    client: Buckets<String>,
    global: Buckets<()>,
}

pub struct Signup {
    cfg: SignupConfig,
    rate: Mutex<Rate>,
    daily: StoredWindows,
    created: AtomicU64,
    refused_rate: AtomicU64,
    refused_invite: AtomicU64,
}

impl Signup {
    pub fn new(cfg: SignupConfig, store: Arc<dyn Store>) -> Arc<Signup> {
        Arc::new(Signup {
            cfg,
            rate: Mutex::new(Rate {
                client: Buckets::per_minute(
                    cfg.per_client_per_min,
                    cfg.per_client_per_min,
                    MAX_TRACKED,
                ),
                // One key; a cap of 1 would clear it on every take.
                global: Buckets::per_minute(cfg.per_min, cfg.per_min / 2.0, 2),
            }),
            daily: StoredWindows::new("reg:", DAY, MAX_TRACKED, store),
            created: AtomicU64::new(0),
            refused_rate: AtomicU64::new(0),
            refused_invite: AtomicU64::new(0),
        })
    }

    pub fn mode(&self) -> Mode {
        self.cfg.mode
    }

    /// May `client` try to make an account now? Charges the per-minute
    /// limits when it may.
    pub fn admit(&self, client: &str, now: Instant) -> Result<(), AppError> {
        if self.cfg.mode == Mode::Closed {
            return Err(AppError::Forbidden(
                "sign-up is closed on this server".into(),
            ));
        }
        let refuse = |wait: Duration, msg: &str| {
            self.refused_rate.fetch_add(1, Ordering::Relaxed);
            Err(AppError::RateLimited(wait, msg.into()))
        };
        if self.daily.count(client, now) >= self.cfg.per_client_per_day {
            return refuse(
                self.daily.resets_in(client, now),
                "too many accounts made from here today",
            );
        }
        let mut r = self.rate.lock().unwrap();
        let key = client.to_string();
        let wait = r.client.wait(&key, now).max(r.global.wait(&(), now));
        if !wait.is_zero() {
            return refuse(wait, "too many sign-ups, wait a minute");
        }
        r.client.take(&key, now);
        r.global.take(&(), now);
        Ok(())
    }

    /// An account was made from `client`.
    pub fn created(&self, client: &str, now: Instant) {
        self.created.fetch_add(1, Ordering::Relaxed);
        self.daily.add(client, now);
    }

    pub fn bad_invite(&self) {
        self.refused_invite.fetch_add(1, Ordering::Relaxed);
    }

    /// Read back today's per-address counts (after a restart).
    pub async fn restore(&self) -> Result<usize, AppError> {
        self.daily.restore().await
    }

    /// Counts only (for `/admin/limits`).
    pub fn stats(&self) -> Value {
        json!({
            "mode": self.cfg.mode.as_str(),
            "created_since_start": self.created.load(Ordering::Relaxed),
            "refused_rate_since_start": self.refused_rate.load(Ordering::Relaxed),
            "refused_invite_since_start": self.refused_invite.load(Ordering::Relaxed),
            "addresses_counted_today": self.daily.len(),
        })
    }
}

/// An invite code as typed: case, spaces and dashes don't matter.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The stored form of an invite code (Argon2id, fixed salt, so it can be
/// looked up; codes are random, 100 bits).
pub fn invite_hash(code: &str) -> Result<String, AppError> {
    let mut out = [0u8; 32];
    Argon2::default()
        .hash_password_into(normalize(code).as_bytes(), b"my-domain/invite/v1", &mut out)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// A new invite code: 20 characters from a 32-letter alphabet, in groups
/// of four.
pub fn new_invite_code() -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut b = [0u8; 20];
    OsRng.fill_bytes(&mut b);
    let s: Vec<char> = b
        .iter()
        .map(|x| ALPHABET[usize::from(x % 32)] as char)
        .collect();
    s.chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemStore;

    fn cfg() -> SignupConfig {
        SignupConfig {
            per_client_per_min: 3.0,
            per_min: 1000.0,
            per_client_per_day: 4,
            ..Default::default()
        }
    }

    #[test]
    fn modes_parse_and_unknown_ones_are_closed() {
        assert_eq!(Mode::parse(None), Mode::Open);
        assert_eq!(Mode::parse(Some(" Invite ")), Mode::Invite);
        assert_eq!(Mode::parse(Some("closed")), Mode::Closed);
        assert_eq!(Mode::parse(Some("opne")), Mode::Closed);
    }

    #[test]
    fn tries_are_limited_per_address_and_server_wide() {
        let s = Signup::new(cfg(), MemStore::new());
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(s.admit("1.2.3.4", t0).is_ok());
        }
        assert!(matches!(
            s.admit("1.2.3.4", t0),
            Err(AppError::RateLimited(..))
        ));
        assert!(s.admit("5.6.7.8", t0).is_ok(), "another address");
        assert!(s.admit("1.2.3.4", t0 + Duration::from_secs(21)).is_ok());

        let g = Signup::new(
            SignupConfig {
                per_min: 4.0,
                ..cfg()
            },
            MemStore::new(),
        );
        assert!(g.admit("a", t0).is_ok());
        assert!(g.admit("b", t0).is_ok());
        assert!(g.admit("c", t0).is_err(), "server-wide burst of 2");
    }

    #[tokio::test]
    async fn the_daily_cap_per_address_survives_a_restart() {
        let store = MemStore::new();
        let s = Signup::new(cfg(), store.clone());
        let mut t = Instant::now();
        for _ in 0..4 {
            assert!(s.admit("1.2.3.4", t).is_ok());
            s.created("1.2.3.4", t);
            t += Duration::from_secs(60);
        }
        let Err(AppError::RateLimited(wait, _)) = s.admit("1.2.3.4", t) else {
            panic!("over the daily cap");
        };
        assert!(wait > Duration::from_secs(23 * 3600));

        // A new server on the same store.
        let after = Signup::new(cfg(), store);
        assert_eq!(after.restore().await.unwrap(), 1);
        assert!(matches!(
            after.admit("1.2.3.4", Instant::now()),
            Err(AppError::RateLimited(..))
        ));
        assert!(after.admit("5.6.7.8", Instant::now()).is_ok());
    }

    #[test]
    fn closed_refuses_everyone() {
        let s = Signup::new(
            SignupConfig {
                mode: Mode::Closed,
                ..cfg()
            },
            MemStore::new(),
        );
        assert!(matches!(
            s.admit("1.2.3.4", Instant::now()),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn invite_codes_hash_the_same_however_they_are_typed() {
        let code = new_invite_code();
        assert_eq!(code.len(), 24);
        let h = invite_hash(&code).unwrap();
        assert_eq!(h.len(), 64);
        assert!(!h.contains(&normalize(&code)));
        assert_eq!(
            invite_hash(&format!(" {} ", code.to_uppercase().replace('-', ""))).unwrap(),
            h
        );
        assert_ne!(invite_hash(&new_invite_code()).unwrap(), h);
    }
}
