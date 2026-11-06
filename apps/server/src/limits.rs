//! Small in-memory limiters shared by the handlers: a token bucket per key, a
//! fixed-window counter per key, and a "seen lately" set. Every map expires
//! idle entries and is capped, so a flood of new keys (addresses, accounts)
//! can't grow server memory without end.
//!
//! Tunables come from the environment (see `apps/server/README.md`, "Limits").

use std::collections::HashMap;
use std::hash::Hash;
use std::str::FromStr;
use std::time::{Duration, Instant};

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

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
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
