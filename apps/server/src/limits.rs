//! Small in-memory limiters shared by the handlers: a token bucket per key, a
//! fixed-window counter per key, and a "seen lately" set. Every map expires
//! idle entries and is capped, so a flood of new keys (addresses, accounts)
//! can't grow server memory without end. `StoredWindows` (daily counters)
//! also writes through to the store, so a restart keeps them.
//!
//! Tunables come from the environment (see `apps/server/README.md`, "Limits").

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::store::{LimitRec, Store};

/// The key a client address is limited under: the address, or for IPv6 its
/// /64 (one subscriber usually holds a whole /64).
pub fn client_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
    }
}

/// Milliseconds since the Unix epoch.
pub fn wall_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `t` as wall-clock ms. Measured from one moment fixed for the process, so
/// the same `Instant` always gives the same value (a window's start names
/// its record).
pub fn to_wall(t: Instant) -> i64 {
    static ANCHOR: std::sync::OnceLock<(Instant, i64)> = std::sync::OnceLock::new();
    let (at, at_ms) = *ANCHOR.get_or_init(|| (Instant::now(), wall_ms()));
    if t >= at {
        at_ms + (t - at).as_millis() as i64
    } else {
        at_ms - (at - t).as_millis() as i64
    }
}

/// Wall-clock `ms` as an `Instant` (`None` if this process can't express
/// it, which only happens for long-gone times).
pub fn from_wall(ms: i64, now: Instant, now_ms: i64) -> Option<Instant> {
    let d = Duration::from_millis(ms.abs_diff(now_ms));
    if ms >= now_ms {
        now.checked_add(d)
    } else {
        now.checked_sub(d)
    }
}

/// `name` from the environment parsed as `T`, else `default` (also when it
/// doesn't parse, or is zero/negative for numbers: limits are never off).
pub fn env_or<T: FromStr + PartialOrd + Default>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<T>().ok())
        .filter(|v| *v > T::default())
        .unwrap_or(default)
}

/// How often the maps below drop idle entries.
const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// Drop the older half of `map` by `age` (the entries touched longest ago),
/// used when a map is still full after its idle entries were dropped.
pub(crate) fn evict_oldest<K: Eq + Hash + Clone, V>(
    map: &mut HashMap<K, V>,
    last: impl Fn(&V) -> Instant,
) {
    let mut ages: Vec<(Instant, K)> = map.iter().map(|(k, v)| (last(v), k.clone())).collect();
    let half = ages.len() / 2;
    if half == 0 {
        map.clear();
        return;
    }
    ages.select_nth_unstable_by_key(half, |(t, _)| *t);
    for (_, k) in ages.into_iter().take(half) {
        map.remove(&k);
    }
}

/// One token bucket per key: `rate` tokens a second, holding at most `burst`.
/// An entry that has refilled to `burst` is the same as no entry, so idle
/// ones are dropped.
pub struct Buckets<K> {
    map: HashMap<K, (f64, Instant)>,
    rate: f64,
    burst: f64,
    max_keys: usize,
    swept: Option<Instant>,
}

impl<K: Eq + Hash + Clone> Buckets<K> {
    /// `per_min` tokens a minute, bursts up to `burst`.
    pub fn per_minute(per_min: f64, burst: f64, max_keys: usize) -> Self {
        Buckets {
            map: HashMap::new(),
            rate: per_min / 60.0,
            burst: burst.max(1.0),
            max_keys,
            swept: None,
        }
    }

    fn level(&self, key: &K, now: Instant) -> f64 {
        match self.map.get(key) {
            Some((tokens, last)) => {
                let dt = now.saturating_duration_since(*last).as_secs_f64();
                (tokens + dt * self.rate).min(self.burst)
            }
            None => self.burst,
        }
    }

    /// Zero if `key` has a token now, else how long until it will.
    pub fn wait(&self, key: &K, now: Instant) -> Duration {
        let t = self.level(key, now);
        if t >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - t) / self.rate)
        }
    }

    /// Take a token from `key` (call after `wait` said zero).
    pub fn take(&mut self, key: &K, now: Instant) {
        self.sweep(now);
        let t = self.level(key, now);
        self.map.insert(key.clone(), ((t - 1.0).max(0.0), now));
    }

    /// `wait`, then `take` when it was zero.
    pub fn try_take(&mut self, key: &K, now: Instant) -> Result<(), Duration> {
        let w = self.wait(key, now);
        if w.is_zero() {
            self.take(key, now);
            Ok(())
        } else {
            Err(w)
        }
    }

    fn sweep(&mut self, now: Instant) {
        let due = self
            .swept
            .is_none_or(|t| now.saturating_duration_since(t) >= SWEEP_EVERY);
        if !due && self.map.len() < self.max_keys {
            return;
        }
        self.swept = Some(now);
        let full_after = Duration::from_secs_f64(self.burst / self.rate);
        self.map
            .retain(|_, (_, last)| now.saturating_duration_since(*last) < full_after);
        if self.map.len() >= self.max_keys {
            evict_oldest(&mut self.map, |(_, last)| *last);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// A count per key over a fixed window that starts at the key's first event.
pub struct Windows<K> {
    map: HashMap<K, (Instant, u32)>,
    window: Duration,
    max_keys: usize,
    swept: Option<Instant>,
}

impl<K: Eq + Hash + Clone> Windows<K> {
    pub fn new(window: Duration, max_keys: usize) -> Self {
        Windows {
            map: HashMap::new(),
            window,
            max_keys,
            swept: None,
        }
    }

    /// Events for `key` in its current window.
    pub fn count(&self, key: &K, now: Instant) -> u32 {
        match self.map.get(key) {
            Some((since, n)) if now.saturating_duration_since(*since) < self.window => *n,
            _ => 0,
        }
    }

    /// How long until `key`'s window ends (zero if it has none).
    pub fn resets_in(&self, key: &K, now: Instant) -> Duration {
        match self.map.get(key) {
            Some((since, _)) => (*since + self.window).saturating_duration_since(now),
            None => Duration::ZERO,
        }
    }

    /// Count one event for `key`.
    pub fn add(&mut self, key: &K, now: Instant) {
        self.sweep(now);
        let e = self.map.entry(key.clone()).or_insert((now, 0));
        if now.saturating_duration_since(e.0) >= self.window {
            *e = (now, 0);
        }
        e.1 = e.1.saturating_add(1);
    }

    fn sweep(&mut self, now: Instant) {
        let due = self
            .swept
            .is_none_or(|t| now.saturating_duration_since(t) >= SWEEP_EVERY);
        if !due && self.map.len() < self.max_keys {
            return;
        }
        self.swept = Some(now);
        let w = self.window;
        self.map
            .retain(|_, (since, _)| now.saturating_duration_since(*since) < w);
        if self.map.len() >= self.max_keys {
            evict_oldest(&mut self.map, |(since, _)| *since);
        }
    }

    /// The window start and count for `key`, if it has a current window.
    pub fn get(&self, key: &K, now: Instant) -> Option<(Instant, u32)> {
        self.map
            .get(key)
            .filter(|(since, _)| now.saturating_duration_since(*since) < self.window)
            .copied()
    }

    /// Put back a window read from storage (after a restart).
    pub fn restore(&mut self, key: K, since: Instant, n: u32) {
        self.map.insert(key, (since, n));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// `Windows` that outlive a restart: every count is written through to the
/// store (one record per key and window, expiring with it) and read back by
/// `restore` at startup. Used for the daily counters.
pub struct StoredWindows {
    prefix: &'static str,
    window: Duration,
    map: Mutex<Windows<String>>,
    store: Arc<dyn Store>,
}

impl StoredWindows {
    pub fn new(
        prefix: &'static str,
        window: Duration,
        max_keys: usize,
        store: Arc<dyn Store>,
    ) -> Self {
        StoredWindows {
            prefix,
            window,
            map: Mutex::new(Windows::new(window, max_keys)),
            store,
        }
    }

    pub fn count(&self, key: &str, now: Instant) -> u32 {
        self.map.lock().unwrap().count(&key.to_string(), now)
    }

    pub fn resets_in(&self, key: &str, now: Instant) -> Duration {
        self.map.lock().unwrap().resets_in(&key.to_string(), now)
    }

    /// Count one event for `key`, here and (in the background) in the store.
    pub fn add(&self, key: &str, now: Instant) {
        let key = key.to_string();
        let (since, n) = {
            let mut m = self.map.lock().unwrap();
            m.add(&key, now);
            m.get(&key, now).unwrap_or((now, 1))
        };
        let since_ms = to_wall(since);
        self.store.put_limit(LimitRec {
            key: format!("{}{key}", self.prefix),
            since_ms,
            n,
            strikes: 0,
            until_ms: 0,
            expires_ms: since_ms + self.window.as_millis() as i64,
        });
    }

    /// Read back the windows still running (the latest per key); how many.
    pub async fn restore(&self) -> Result<usize, crate::error::AppError> {
        let recs = self.store.load_limits(self.prefix).await?;
        let (now, now_ms) = (Instant::now(), wall_ms());
        let mut m = self.map.lock().unwrap();
        let mut restored = 0;
        for r in recs {
            if r.expires_ms <= now_ms {
                continue;
            }
            let Some(since) = from_wall(r.since_ms, now, now_ms) else {
                continue;
            };
            let Some(key) = r.key.strip_prefix(self.prefix).map(str::to_string) else {
                continue;
            };
            let newer = match m.get(&key, now) {
                Some((s, n)) => since > s || (since == s && r.n > n),
                None => true,
            };
            if newer {
                m.restore(key, since, r.n);
                restored += 1;
            }
        }
        Ok(restored)
    }

    /// Keys with a window (for `/admin/limits`).
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }
}

/// Keys seen within `ttl` (for dropping a repeat of the same push).
pub struct Recent<K> {
    map: HashMap<K, Instant>,
    ttl: Duration,
    max_keys: usize,
    swept: Option<Instant>,
}

impl<K: Eq + Hash + Clone> Recent<K> {
    pub fn new(ttl: Duration, max_keys: usize) -> Self {
        Recent {
            map: HashMap::new(),
            ttl,
            max_keys,
            swept: None,
        }
    }

    pub fn seen(&self, key: &K, now: Instant) -> bool {
        self.map
            .get(key)
            .is_some_and(|t| now.saturating_duration_since(*t) < self.ttl)
    }

    pub fn insert(&mut self, key: K, now: Instant) {
        let due = self
            .swept
            .is_none_or(|t| now.saturating_duration_since(t) >= SWEEP_EVERY);
        if due || self.map.len() >= self.max_keys {
            self.swept = Some(now);
            let ttl = self.ttl;
            self.map
                .retain(|_, t| now.saturating_duration_since(*t) < ttl);
            if self.map.len() >= self.max_keys {
                evict_oldest(&mut self.map, |t| *t);
            }
        }
        self.map.insert(key, now);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_allows_its_burst_then_its_rate_and_says_how_long_to_wait() {
        let mut b = Buckets::per_minute(60.0, 3.0, 100);
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(b.try_take(&"a", t0).is_ok());
        }
        let w = b.try_take(&"a", t0).unwrap_err();
        assert!(w > Duration::from_millis(900) && w <= Duration::from_secs(1));
        assert!(b.try_take(&"b", t0).is_ok(), "keys are separate");
        assert!(b.try_take(&"a", t0 + Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn idle_buckets_and_windows_are_dropped_and_the_maps_stay_capped() {
        let t0 = Instant::now();
        let mut b = Buckets::per_minute(60.0, 2.0, 1000);
        for i in 0..500u32 {
            b.take(&i, t0);
        }
        // Refilled (2 s) and past a sweep: gone.
        b.take(&9999, t0 + Duration::from_secs(31));
        assert_eq!(b.len(), 1);
        let mut small = Buckets::per_minute(1.0, 5.0, 100);
        for i in 0..10_000u32 {
            small.take(&i, t0);
        }
        assert!(small.len() <= 100, "{}", small.len());

        let mut w = Windows::new(Duration::from_secs(60), 50);
        for i in 0..10_000u32 {
            w.add(&i, t0);
        }
        assert!(w.len() <= 50);
        w.add(&1, t0 + Duration::from_secs(61));
        assert_eq!(w.len(), 1);

        let mut r = Recent::new(Duration::from_secs(10), 50);
        for i in 0..10_000u32 {
            r.insert(i, t0);
        }
        assert!(r.len() <= 50);
        r.insert(1, t0 + Duration::from_secs(40));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn a_window_counts_until_it_ends() {
        let mut w = Windows::new(Duration::from_secs(60), 10);
        let t0 = Instant::now();
        w.add(&"a", t0);
        w.add(&"a", t0 + Duration::from_secs(10));
        assert_eq!(w.count(&"a", t0 + Duration::from_secs(10)), 2);
        assert_eq!(
            w.resets_in(&"a", t0 + Duration::from_secs(10)),
            Duration::from_secs(50)
        );
        assert_eq!(w.count(&"a", t0 + Duration::from_secs(60)), 0);
    }

    #[test]
    fn env_values_that_are_missing_bad_or_zero_keep_the_default() {
        assert_eq!(env_or("MD_TEST_UNSET_LIMIT_XYZ", 7u32), 7);
        std::env::set_var("MD_TEST_ZERO_LIMIT_XYZ", "0");
        assert_eq!(env_or("MD_TEST_ZERO_LIMIT_XYZ", 7u32), 7);
        std::env::set_var("MD_TEST_SET_LIMIT_XYZ", " 12 ");
        assert_eq!(env_or("MD_TEST_SET_LIMIT_XYZ", 7u32), 12);
    }
}
