//! In-memory token buckets and concurrency slots keyed by client address.
//!
//! IPv4 addresses are keys of their own; IPv6 clients are keyed by their
//! /64. Holding many /64s is cheap (a free tunnel gives a /48, which is
//! 65,536 of them), so the tiered limiters and the slots also charge the
//! client's /56 and /48 with larger budgets.
//!
//! Each limiter holds at most `MAX_ENTRIES` keys. Full (idle) buckets carry
//! no information and are pruned every minute. When the map is still full,
//! a new key evicts the fullest of a few randomly sampled buckets, that is
//! the one that holds its client back least. New clients are therefore never
//! refused just because the map is full, and the buckets that actually
//! restrict someone are the last to go.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub type Key = u128;

pub const MAX_ENTRIES: usize = 100_000;
/// Buckets looked at to choose one to evict when a map is full.
const EVICT_SAMPLE: usize = 8;

/// IPv4 (and IPv4-mapped IPv6) addresses map to themselves, other IPv6
/// addresses to their /64 network.
pub fn ip_key(ip: IpAddr) -> Key {
    match ip {
        IpAddr::V4(v4) => (0xffff_u128 << 32) | u128::from(u32::from(v4)),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => ip_key(IpAddr::V4(v4)),
            None => net(u128::from(v6), 64),
        },
    }
}

fn is_v4(key: Key) -> bool {
    key >> 32 == 0xffff
}

/// The first `bits` bits of `key` (bits < 128).
fn net(key: Key, bits: u32) -> Key {
    key & !(Key::MAX >> bits)
}

/// Inclusive key range of the network of `bits` bits around `key`.
fn net_range(key: Key, bits: u32) -> (Key, Key) {
    let lo = net(key, bits);
    (lo, lo | (Key::MAX >> bits))
}

/// The client key and, for IPv6, its /56 and /48 networks.
pub fn tiers(key: Key) -> [Option<Key>; 3] {
    if is_v4(key) {
        [Some(key), None, None]
    } else {
        [Some(key), Some(net(key, 56)), Some(net(key, 48))]
    }
}

/// Like [`tiers`], as inclusive ranges of client keys (for queries over
/// stored keys).
pub fn tier_ranges(key: Key) -> [Option<(Key, Key)>; 3] {
    if is_v4(key) {
        [Some((key, key)), None, None]
    } else {
        [
            Some((key, key)),
            Some(net_range(key, 56)),
            Some(net_range(key, 48)),
        ]
    }
}

/// The widest network a client is charged for: its /48, or its IPv4 address.
pub fn widest(key: Key) -> Key {
    if is_v4(key) { key } else { net(key, 48) }
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

struct Inner {
    /// Dense storage so that eviction can sample at random.
    slots: Vec<(Key, Bucket)>,
    index: HashMap<Key, usize>,
    last_prune: Option<Instant>,
    seed: u64,
}

impl Inner {
    fn remove_at(&mut self, i: usize) {
        if i >= self.slots.len() {
            return;
        }
        let (key, _) = self.slots.swap_remove(i);
        self.index.remove(&key);
        if let Some((moved, _)) = self.slots.get(i) {
            self.index.insert(*moved, i);
        }
    }

    fn random(&mut self) -> u64 {
        // xorshift64*: only used to pick eviction samples
        let mut x = self.seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn random_seed() -> u64 {
    let s = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    s | 1
}

pub struct Limiter {
    burst: f64,
    per_token: f64,
    max_entries: usize,
    inner: Mutex<Inner>,
}

impl Limiter {
    /// `burst` tokens, one more every `per_token`.
    pub fn new(burst: u32, per_token: Duration) -> Self {
        Self::with_capacity(burst, per_token, MAX_ENTRIES)
    }

    pub fn with_capacity(burst: u32, per_token: Duration, max_entries: usize) -> Self {
        Limiter {
            burst: f64::from(burst.max(1)),
            per_token: per_token.as_secs_f64().max(0.001),
            max_entries: max_entries.max(1),
            inner: Mutex::new(Inner {
                slots: Vec::new(),
                index: HashMap::new(),
                last_prune: None,
                seed: random_seed(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tokens of `b` at `now`, capped at the burst.
    fn level(&self, b: &Bucket, now: Instant) -> f64 {
        let dt = now.saturating_duration_since(b.at).as_secs_f64();
        (b.tokens + dt / self.per_token).min(self.burst)
    }

    fn wait_secs(&self, tokens: f64) -> u64 {
        let secs = ((1.0 - tokens) * self.per_token).ceil();
        if secs.is_finite() && secs >= 1.0 {
            secs as u64
        } else {
            1
        }
    }

    /// Ok when at least one token is available (consumes nothing).
    pub fn check(&self, key: Key) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    /// Consumes one token, or tells how many seconds until one is available.
    pub fn take(&self, key: Key) -> Result<(), u64> {
        self.take_at(key, Instant::now())
    }

    /// Consumes one token if there is one (used to charge failures after the
    /// fact; never blocks).
    pub fn charge(&self, key: Key) {
        let _ = self.take_at(key, Instant::now());
    }

    /// Gives back a token taken by [`Limiter::take`]. Taking first and
    /// refunding on success keeps concurrent attempts from all slipping
    /// through a check that only charges afterwards.
    pub fn refund(&self, key: Key) {
        let mut inner = self.lock();
        let Some(&i) = inner.index.get(&key) else {
            return;
        };
        if let Some((_, b)) = inner.slots.get_mut(i) {
            b.tokens = (b.tokens + 1.0).min(self.burst);
        }
    }

    pub fn check_at(&self, key: Key, now: Instant) -> Result<(), u64> {
        let inner = self.lock();
        let Some(&i) = inner.index.get(&key) else {
            return Ok(());
        };
        match inner.slots.get(i) {
            Some((_, b)) => {
                let t = self.level(b, now);
                if t >= 1.0 {
                    Ok(())
                } else {
                    Err(self.wait_secs(t))
                }
            }
            None => Ok(()),
        }
    }

    pub fn take_at(&self, key: Key, now: Instant) -> Result<(), u64> {
        let mut inner = self.lock();
        if let Some(&i) = inner.index.get(&key)
            && let Some((_, b)) = inner.slots.get_mut(i)
        {
            let t = self.level(b, now);
            b.at = now;
            if t >= 1.0 {
                b.tokens = t - 1.0;
                return Ok(());
            }
            b.tokens = t;
            return Err(self.wait_secs(t));
        }
        if inner.slots.len() >= self.max_entries {
            let recently = inner
                .last_prune
                .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(1));
            if !recently {
                self.prune_locked(&mut inner, now);
            }
            while inner.slots.len() >= self.max_entries {
                self.evict_locked(&mut inner, now);
            }
        }
        let bucket = Bucket {
            tokens: self.burst - 1.0,
            at: now,
        };
        let i = inner.slots.len();
        inner.slots.push((key, bucket));
        inner.index.insert(key, i);
        Ok(())
    }

    /// Drops the fullest of a few random buckets (oldest on a tie).
    fn evict_locked(&self, inner: &mut Inner, now: Instant) {
        let n = inner.slots.len();
        if n == 0 {
            return;
        }
        let mut best: Option<(usize, f64, Instant)> = None;
        for s in 0..EVICT_SAMPLE.min(n) {
            // small maps are scanned whole, large ones sampled
            let i = if n <= EVICT_SAMPLE {
                s
            } else {
                usize::try_from(inner.random() % n as u64).unwrap_or(0)
            };
            let Some((_, b)) = inner.slots.get(i) else {
                continue;
            };
            let t = self.level(b, now);
            let better = match best {
                None => true,
                Some((_, bt, bat)) => t > bt || (t == bt && b.at < bat),
            };
            if better {
                best = Some((i, t, b.at));
            }
        }
        inner.remove_at(best.map_or(0, |(i, ..)| i));
    }

    fn prune_locked(&self, inner: &mut Inner, now: Instant) {
        inner.slots.retain(|(_, b)| self.level(b, now) < self.burst);
        let rebuilt: HashMap<Key, usize> = inner
            .slots
            .iter()
            .enumerate()
            .map(|(i, (k, _))| (*k, i))
            .collect();
        inner.index = rebuilt;
        inner.last_prune = Some(now);
    }

    /// Drops buckets that have refilled completely.
    pub fn prune(&self) {
        self.prune_at(Instant::now());
    }

    pub fn prune_at(&self, now: Instant) {
        let mut inner = self.lock();
        self.prune_locked(&mut inner, now);
    }

    pub fn len(&self) -> usize {
        self.lock().slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A limiter for a client and, for IPv6, its /56 and /48 networks. A take
/// succeeds only when every level has a token.
pub struct Tiered {
    levels: [Limiter; 3],
}

impl Tiered {
    /// (burst, one token per) for the client key, its /56 and its /48.
    pub fn new(levels: [(u32, Duration); 3]) -> Self {
        Tiered {
            levels: levels.map(|(burst, per)| Limiter::new(burst, per)),
        }
    }

    pub fn take(&self, key: Key) -> Result<(), u64> {
        self.take_at(key, Instant::now())
    }

    pub fn take_at(&self, key: Key, now: Instant) -> Result<(), u64> {
        let keys = tiers(key);
        for (n, (l, k)) in self.levels.iter().zip(keys).enumerate() {
            let Some(k) = k else {
                break;
            };
            if let Err(wait) = l.take_at(k, now) {
                for (l2, k2) in self.levels.iter().zip(keys).take(n) {
                    if let Some(k2) = k2 {
                        l2.refund(k2);
                    }
                }
                return Err(wait);
            }
        }
        Ok(())
    }

    pub fn refund(&self, key: Key) {
        for (l, k) in self.levels.iter().zip(tiers(key)) {
            if let Some(k) = k {
                l.refund(k);
            }
        }
    }

    pub fn check(&self, key: Key) -> Result<(), u64> {
        let mut wait = None;
        for (l, k) in self.levels.iter().zip(tiers(key)) {
            if let Some(k) = k
                && let Err(w) = l.check(k)
            {
                wait = Some(wait.map_or(w, |x: u64| x.max(w)));
            }
        }
        wait.map_or(Ok(()), Err)
    }

    pub fn prune(&self) {
        for l in &self.levels {
            l.prune();
        }
    }
}

/// A shared budget that cannot be used to lock everyone out.
///
/// While the shared bucket (one per `scope`) has tokens, nothing more is
/// asked. Once it is empty, which takes a distributed attack, each client
/// network (its /48 or IPv4 address) may still spend one token per strict
/// period. A receiver who has not failed recently therefore still gets
/// through, while a prober that has just failed is held back. Refusing every
/// lookup instead would hand the attacker a switch that turns the service
/// off for everybody.
pub struct Guarded {
    shared: Limiter,
    strict: Limiter,
}

/// Which bucket paid, so that a success can be refunded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ticket {
    Shared(Key),
    Strict(Key),
}

impl Guarded {
    pub fn new(shared: (u32, Duration), strict_period: Duration) -> Self {
        Guarded {
            shared: Limiter::new(shared.0, shared.1),
            strict: Limiter::new(1, strict_period),
        }
    }

    pub fn take(&self, scope: Key, client: Key) -> Result<Ticket, u64> {
        self.take_at(scope, client, Instant::now())
    }

    pub fn take_at(&self, scope: Key, client: Key, now: Instant) -> Result<Ticket, u64> {
        if self.shared.take_at(scope, now).is_ok() {
            return Ok(Ticket::Shared(scope));
        }
        let w = widest(client);
        self.strict.take_at(w, now).map(|()| Ticket::Strict(w))
    }

    pub fn refund(&self, t: Ticket) {
        match t {
            Ticket::Shared(k) => self.shared.refund(k),
            Ticket::Strict(k) => self.strict.refund(k),
        }
    }

    pub fn prune(&self) {
        self.shared.prune();
        self.strict.prune();
    }
}

// ---------------------------------------------------------------------------
// Concurrency slots
// ---------------------------------------------------------------------------

/// Open connections per client: IPv4 address or IPv6 /64, /56, /48. Only
/// enforced without a trusted proxy, where the peer address is the client.
pub const CONNS_PER_CLIENT: [u32; 3] = [64, 128, 256];
/// Downloads streaming at once per client.
pub const DOWNLOADS_PER_CLIENT: [u32; 3] = [16, 32, 64];
/// File uploads (PUT) receiving at once per client.
pub const UPLOADS_PER_CLIENT: [u32; 3] = [12, 24, 48];
/// Large JSON bodies (parcel creation above 64 KiB) read at once per client.
pub const BIG_BODIES_PER_CLIENT: [u32; 3] = [2, 4, 8];

/// Counts things in progress (connections, transfers) per client and per
/// wider IPv6 network, with a cap at each level.
pub struct Slots {
    caps: [u32; 3],
    inner: Mutex<HashMap<(u8, Key), u32>>,
}

/// One slot; given back on drop.
pub struct SlotGuard {
    slots: Arc<Slots>,
    keys: [Option<Key>; 3],
}

const LEVELS: [u8; 3] = [0, 1, 2];

impl Slots {
    pub fn new(caps: [u32; 3]) -> Arc<Slots> {
        Arc::new(Slots {
            caps,
            inner: Mutex::new(HashMap::new()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(u8, Key), u32>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A slot for `key`, or `None` when the client or one of its networks
    /// is at its cap.
    pub fn try_acquire(self: &Arc<Self>, key: Key) -> Option<SlotGuard> {
        let keys = tiers(key);
        let mut m = self.lock();
        for ((lvl, k), cap) in LEVELS.iter().zip(keys).zip(self.caps) {
            if let Some(k) = k
                && m.get(&(*lvl, k)).copied().unwrap_or(0) >= cap
            {
                return None;
            }
        }
        for (lvl, k) in LEVELS.iter().zip(keys) {
            if let Some(k) = k {
                let n = m.entry((*lvl, k)).or_insert(0);
                *n = n.saturating_add(1);
            }
        }
        drop(m);
        Some(SlotGuard {
            slots: Arc::clone(self),
            keys,
        })
    }

    /// Slots held by exactly this client key.
    pub fn active(&self, key: Key) -> u32 {
        self.lock().get(&(0, key)).copied().unwrap_or(0)
    }
}

/// The server's concurrency caps (see the constants above).
pub struct ClientSlots {
    pub conns: Arc<Slots>,
    pub downloads: Arc<Slots>,
    pub uploads: Arc<Slots>,
    pub bodies: Arc<Slots>,
}

impl ClientSlots {
    pub fn new() -> Self {
        ClientSlots {
            conns: Slots::new(CONNS_PER_CLIENT),
            downloads: Slots::new(DOWNLOADS_PER_CLIENT),
            uploads: Slots::new(UPLOADS_PER_CLIENT),
            bodies: Slots::new(BIG_BODIES_PER_CLIENT),
        }
    }
}

impl Default for ClientSlots {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let mut m = self.slots.lock();
        for (lvl, k) in LEVELS.iter().zip(self.keys) {
            let Some(k) = k else {
                continue;
            };
            if let Some(n) = m.get_mut(&(*lvl, k)) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(&(*lvl, k));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The server's limiters
// ---------------------------------------------------------------------------

const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Seconds a strict ticket lasts (see [`Guarded`]).
pub const STRICT_PERIOD: Duration = secs(600);

/// All limiters of the server (see spec section 7). Tiered entries list the
/// (burst, one token per) of the client, its /56 and its /48.
pub struct RateLimits {
    /// Every code lookup (info and claim).
    pub lookup: Limiter,
    /// Lookups of codes that do not exist (enumeration brake).
    pub miss: Tiered,
    /// Server wide miss budget (see [`Guarded`]).
    pub miss_global: Guarded,
    /// Wrong parcel passwords per client.
    pub pw_ip: Tiered,
    /// Wrong parcel passwords per parcel (keyed by parcel id).
    pub pw_parcel: Limiter,
    /// Failed logins per client.
    pub login_fail: Tiered,
    /// Failed logins per account (keyed by account id; see [`Guarded`]).
    pub login_account: Guarded,
    /// Account creation per client.
    pub register: Tiered,
    /// Parcel creation per client.
    pub create: Tiered,
    /// QR code renders per client.
    pub qr: Tiered,
}

impl RateLimits {
    pub fn new() -> Self {
        RateLimits {
            lookup: Limiter::new(30, secs(2)),
            miss: Tiered::new([(10, secs(10)), (30, secs(5)), (60, secs(2))]),
            miss_global: Guarded::new((300, Duration::from_millis(500)), STRICT_PERIOD),
            pw_ip: Tiered::new([(8, secs(30)), (24, secs(15)), (48, secs(10))]),
            pw_parcel: Limiter::new(20, secs(60)),
            login_fail: Tiered::new([(8, secs(30)), (24, secs(15)), (48, secs(10))]),
            login_account: Guarded::new((10, secs(60)), STRICT_PERIOD),
            register: Tiered::new([(5, secs(600)), (10, secs(300)), (20, secs(120))]),
            create: Tiered::new([(30, secs(5)), (60, secs(3)), (120, secs(2))]),
            qr: Tiered::new([
                (20, secs(2)),
                (40, secs(1)),
                (80, Duration::from_millis(500)),
            ]),
        }
    }

    pub fn prune(&self) {
        self.lookup.prune();
        self.miss.prune();
        self.miss_global.prune();
        self.pw_ip.prune();
        self.pw_parcel.prune();
        self.login_fail.prune();
        self.login_account.prune();
        self.register.prune();
        self.create.prune();
        self.qr.prune();
    }
}

impl Default for RateLimits {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn v6(s: &str) -> Key {
        ip_key(IpAddr::V6(s.parse::<Ipv6Addr>().expect("ip")))
    }

    #[test]
    fn bucket_burst_and_refill() {
        let l = Limiter::new(3, secs(10));
        let t0 = Instant::now();
        assert!(l.check_at(1, t0).is_ok());
        assert!(l.take_at(1, t0).is_ok());
        assert!(l.take_at(1, t0).is_ok());
        assert!(l.take_at(1, t0).is_ok());
        assert_eq!(l.take_at(1, t0), Err(10));
        assert_eq!(l.check_at(1, t0), Err(10));
        // other keys are independent
        assert!(l.take_at(2, t0).is_ok());
        // 4 s later still 6 s to wait
        assert_eq!(l.take_at(1, t0 + secs(4)), Err(6));
        // 10 s later one token
        assert!(l.take_at(1, t0 + secs(10)).is_ok());
        assert!(l.take_at(1, t0 + secs(10)).is_err());
        // refill caps at burst
        let later = t0 + secs(1000);
        for _ in 0..3 {
            assert!(l.take_at(1, later).is_ok());
        }
        assert!(l.take_at(1, later).is_err());
        // refunds restore a token, never above burst
        l.refund(1);
        assert!(l.take_at(1, later).is_ok());
        for _ in 0..10 {
            l.refund(1);
        }
        for _ in 0..3 {
            assert!(l.take_at(1, later).is_ok());
        }
        assert!(l.take_at(1, later).is_err());
        // sub second refill
        let fast = Limiter::new(1, Duration::from_millis(250));
        assert!(fast.take_at(9, t0).is_ok());
        assert_eq!(fast.take_at(9, t0), Err(1));
        assert!(fast.take_at(9, t0 + Duration::from_millis(250)).is_ok());
    }

    #[test]
    fn full_map_evicts_instead_of_refusing() {
        let l = Limiter::with_capacity(5, secs(100), 4);
        let t0 = Instant::now();
        // key 1 is held back (empty bucket); 2, 3, 4 are barely used
        for _ in 0..5 {
            assert!(l.take_at(1, t0).is_ok());
        }
        for k in 2..5 {
            assert!(l.take_at(k, t0).is_ok());
        }
        assert_eq!(l.len(), 4);
        // map full, nothing refilled: new keys still get in
        for k in 10..40 {
            assert!(l.take_at(k, t0).is_ok(), "new key {k} refused");
            assert_eq!(l.len(), 4);
            // the restricted client is never the one evicted while fuller
            // buckets are around
            assert!(l.take_at(1, t0).is_err(), "restricted key lost at {k}");
        }
        // after refill the idle entries are pruned
        let t1 = t0 + secs(1000);
        l.prune_at(t1);
        assert!(l.is_empty());
    }

    #[test]
    fn tiers_charge_wider_networks() {
        let t = Tiered::new([(2, secs(100)), (3, secs(100)), (4, secs(100))]);
        let now = Instant::now();
        let a = v6("2001:db8:1:100::1");
        let b = v6("2001:db8:1:101::1"); // same /56 as a
        let c = v6("2001:db8:1:200::1"); // same /48, other /56
        assert!(t.take_at(a, now).is_ok());
        assert!(t.take_at(a, now).is_ok());
        assert!(t.take_at(a, now).is_err()); // /64 empty
        assert!(t.take_at(b, now).is_ok()); // /56 now empty
        assert!(t.take_at(b, now).is_err());
        assert!(t.take_at(c, now).is_ok()); // /48 now empty
        assert!(t.take_at(v6("2001:db8:1:300::1"), now).is_err());
        // a refused take charged nothing
        assert!(t.take_at(v6("2001:db8:2::1"), now).is_ok());
        // IPv4 clients are one level only
        let v4 = ip_key(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(tiers(v4), [Some(v4), None, None]);
        assert!(t.take_at(v4, now).is_ok());
        assert!(t.take_at(v4, now).is_ok());
        assert!(t.take_at(v4, now).is_err());
    }

    #[test]
    fn guarded_budget_never_locks_out_clean_clients() {
        let g = Guarded::new((2, secs(1000)), secs(600));
        let now = Instant::now();
        let x = v6("2001:db8:a::1");
        let y = v6("2001:db8:b::1");
        assert_eq!(g.take_at(0, x, now), Ok(Ticket::Shared(0)));
        assert_eq!(g.take_at(0, x, now), Ok(Ticket::Shared(0)));
        // shared budget empty: one strict ticket per network
        let tx = g.take_at(0, x, now).expect("strict ticket");
        assert_eq!(tx, Ticket::Strict(widest(x)));
        assert!(g.take_at(0, x, now).is_err());
        assert!(g.take_at(0, v6("2001:db8:a:ffff::9"), now).is_err()); // same /48
        assert!(g.take_at(0, y, now).is_ok());
        // a success hands the ticket back
        g.refund(tx);
        assert!(g.take_at(0, x, now).is_ok());
    }

    #[test]
    fn slots_cap_each_level() {
        let s = Slots::new([2, 3, 4]);
        let a = v6("2001:db8:1:100::1");
        let g1 = s.try_acquire(a).expect("slot");
        let g2 = s.try_acquire(a).expect("slot");
        assert!(s.try_acquire(a).is_none());
        assert_eq!(s.active(a), 2);
        let g3 = s.try_acquire(v6("2001:db8:1:1ff::1")).expect("same /56");
        assert!(s.try_acquire(v6("2001:db8:1:1fe::1")).is_none());
        let g4 = s.try_acquire(v6("2001:db8:1:200::1")).expect("same /48");
        assert!(s.try_acquire(v6("2001:db8:1:300::1")).is_none());
        drop((g1, g2, g3, g4));
        assert_eq!(s.active(a), 0);
        assert!(s.lock().is_empty());
    }

    #[test]
    fn ip_keys() {
        let a = ip_key(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
        let mapped = ip_key(IpAddr::V6(Ipv4Addr::new(1, 2, 3, 4).to_ipv6_mapped()));
        assert_eq!(a, mapped);
        assert_eq!(widest(a), a);
        let x = v6("2001:db8:1:2:aaaa::1");
        let y = v6("2001:db8:1:2:ffff::9");
        let z = v6("2001:db8:1:3::1");
        assert_eq!(x, y);
        assert_ne!(x, z);
        assert_eq!(widest(x), widest(z));
        assert_eq!(
            tier_ranges(z)[2],
            Some((
                v6("2001:db8:1::"),
                u128::from(
                    "2001:db8:1:ffff:ffff:ffff:ffff:ffff"
                        .parse::<Ipv6Addr>()
                        .expect("ip")
                )
            ))
        );
        // a /64 key never looks like an IPv4 key
        assert!(!is_v4(x));
        assert!(is_v4(a));
    }
}
