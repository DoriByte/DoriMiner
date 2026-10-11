//! The optional dev fee: a share of the time mined for the DoriByte wallet, on the
//! same pool the user chose. Optional: 1% unless the user says
//! otherwise (`--dev-fee PERCENT`; `--dev-fee 0` turns it off).
//!
//! The GPUs mine for the user until a window is due; then a second pool link logs
//! in with the DoriByte wallet, the GPUs take its jobs for `WINDOW`, and the user's
//! link, which never went down, gets them back. A window is never allowed to cost
//! the user more than its own length: it does not start while the user's pool is
//! down, it is skipped when the dev fee link does not come up, and it ends the
//! moment that link fails.
//!
//! The fee is never more than the percentage, whatever the length of the run: a
//! window comes only after the whole of its gap was mined for the user (99 minutes
//! for 1%), the first one included, and only the time the user's pool link was up
//! counts towards the gap. A run shorter than the gap has no window at all, and
//! nothing is carried from one run to the next.

use crate::state::Shared;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// The DoriByte wallet for Quantus. Public, and the same one Dori Rig shows.
pub const DEV_WALLET: &str = "qzjcpLMMxF6D7d2vZeMwJDxdi5zRSe9xCJ1Dpmvf6Nt1zD4aM";
/// The worker name on the DoriByte account: the user's own stays out of it.
pub const DEV_WORKER: &str = "doriminer";
/// The fee when the user gives none.
pub const DEFAULT_PERCENT: u64 = 1;
/// The highest fee that can be asked for.
pub const MAX_PERCENT: u64 = 5;

/// How long one dev fee window lasts.
pub const WINDOW: Duration = Duration::from_secs(60);
/// A window is closed this much before `WINDOW` is up: the main loop looks at the
/// clock a few times a second, and a look that comes late must not make the fee
/// more than its percentage.
const MARGIN: Duration = Duration::from_secs(2);
/// The password of the dev fee link. The user's own is not sent on it.
pub const DEV_PASSWORD: &str = "x";
/// How long the dev fee link has to log in and get a job before the window is skipped.
const CONNECT_WAIT: Duration = Duration::from_secs(60);
/// After a skipped window, the next try.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The login of the dev fee link, in the form of the user's: `wallet/worker`
/// (Kryptex), `wallet.worker`, or the wallet alone.
pub fn dev_login(user_login: &str) -> String {
    if user_login.contains('/') {
        format!("{DEV_WALLET}/{DEV_WORKER}")
    } else if user_login.contains('.') {
        format!("{DEV_WALLET}.{DEV_WORKER}")
    } else {
        DEV_WALLET.to_string()
    }
}

/// Mining for the user between two windows, so that `WINDOW` is `percent` of the whole.
fn gap(percent: u64) -> Duration {
    WINDOW * (100 - percent as u32) / percent as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// For the user, until this much more was mined with the user's pool link up.
    User { left: Duration },
    /// The dev fee link was asked for; the GPUs are still the user's.
    Connecting { since: Instant },
    /// The GPUs are on the dev fee link's jobs.
    Window { since: Instant },
}

pub struct Schedule {
    percent: u64,
    state: State,
    last_tick: Instant,
}

impl Schedule {
    /// `None` when the fee is off.
    pub fn new(shared: &Shared, now: Instant) -> Option<Self> {
        let percent = shared.fee.percent.load(Ordering::Relaxed);
        (percent > 0).then(|| Self {
            percent,
            state: State::User { left: gap(percent) },
            last_tick: now,
        })
    }

    /// Called a few times a second by the main loop.
    pub fn tick(&mut self, shared: &Shared, now: Instant) {
        let fee = &shared.fee;
        let passed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        match self.state {
            State::User { left } => {
                // Time with the user's pool down was not mined for the user.
                if !shared.pool_connected.load(Ordering::SeqCst) {
                    return;
                }
                let left = left.saturating_sub(passed);
                if left.is_zero() {
                    fee.wanted.store(true, Ordering::SeqCst);
                    self.state = State::Connecting { since: now };
                } else {
                    self.state = State::User { left };
                }
            }
            State::Connecting { since } => {
                if fee.ready.load(Ordering::SeqCst) {
                    fee.active.store(true, Ordering::SeqCst);
                    self.state = State::Window { since: now };
                    log::info!(
                        "dev fee: mining for DoriByte for {} s ({}% of the time; --dev-fee 0 turns it off)",
                        (WINDOW - MARGIN).as_secs(),
                        self.percent
                    );
                } else if now.duration_since(since) >= CONNECT_WAIT {
                    fee.wanted.store(false, Ordering::SeqCst);
                    self.state = State::User { left: RETRY_AFTER };
                    log::warn!("dev fee: the link did not come up, window skipped");
                }
            }
            State::Window { since } => {
                let done = now.duration_since(since) + MARGIN >= WINDOW;
                if done || !fee.ready.load(Ordering::SeqCst) {
                    fee.active.store(false, Ordering::SeqCst);
                    fee.wanted.store(false, Ordering::SeqCst);
                    fee.seconds
                        .fetch_add(now.duration_since(since).as_secs(), Ordering::Relaxed);
                    // The user's link puts its job back when it sees the turn
                    // change; if it is down there is nobody to do it.
                    if !shared.pool_connected.load(Ordering::SeqCst) {
                        shared.clear_work();
                    }
                    self.state = State::User {
                        left: gap(self.percent),
                    };
                    log::info!(
                        "dev fee: window over ({} shares accepted so far), back to your wallet",
                        fee.accepted.load(Ordering::Relaxed)
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(percent: u64) -> Shared {
        let shared = Shared::new("stratum+tcp://pool:1".into(), Vec::new());
        shared.fee.percent.store(percent, Ordering::Relaxed);
        shared.pool_connected.store(true, Ordering::SeqCst);
        shared
    }
    /// (the link is wanted, the GPUs are on it)
    fn flags(shared: &Shared) -> (bool, bool) {
        (
            shared.fee.wanted.load(Ordering::SeqCst),
            shared.fee.active.load(Ordering::SeqCst),
        )
    }

    #[test]
    fn off_means_no_schedule() {
        assert!(Schedule::new(&shared(0), Instant::now()).is_none());
    }

    #[test]
    fn one_percent_is_one_minute_in_a_hundred() {
        assert_eq!(gap(1) + WINDOW, WINDOW * 100);
        assert_eq!(gap(5) + WINDOW, WINDOW * 20);
    }

    #[test]
    fn the_login_follows_the_form_of_the_users() {
        assert_eq!(dev_login("qzabc/rig"), format!("{DEV_WALLET}/{DEV_WORKER}"));
        assert_eq!(dev_login("qzabc.rig"), format!("{DEV_WALLET}.{DEV_WORKER}"));
        assert_eq!(dev_login("qzabc"), DEV_WALLET);
    }

    #[test]
    fn a_short_run_has_no_window() {
        let shared = shared(1);
        let start = Instant::now();
        let mut schedule = Schedule::new(&shared, start).unwrap();
        schedule.tick(&shared, start + gap(1) - Duration::from_secs(1));
        assert_eq!(flags(&shared), (false, false));
    }

    #[test]
    fn a_window_opens_when_the_link_is_ready_and_closes_on_time() {
        let shared = shared(1);
        let start = Instant::now();
        let mut schedule = Schedule::new(&shared, start).unwrap();
        let due = start + gap(1);
        schedule.tick(&shared, due);
        assert_eq!(flags(&shared), (true, false), "the link is asked for, the GPUs stay");
        schedule.tick(&shared, due + Duration::from_secs(5));
        assert_eq!(flags(&shared), (true, false), "not ready yet");
        shared.fee.ready.store(true, Ordering::SeqCst);
        let open = due + Duration::from_secs(8);
        schedule.tick(&shared, open);
        assert_eq!(flags(&shared), (true, true));
        let close = open + WINDOW - MARGIN;
        schedule.tick(&shared, close - Duration::from_secs(1));
        assert_eq!(flags(&shared), (true, true));
        schedule.tick(&shared, close);
        assert_eq!(flags(&shared), (false, false));
        assert_eq!(shared.fee.seconds.load(Ordering::Relaxed), (WINDOW - MARGIN).as_secs());
        // The next one is a whole gap away.
        shared.fee.ready.store(false, Ordering::SeqCst);
        schedule.tick(&shared, close + gap(1) - Duration::from_secs(1));
        assert_eq!(flags(&shared), (false, false));
        schedule.tick(&shared, close + gap(1));
        assert_eq!(flags(&shared), (true, false));
    }

    #[test]
    fn no_window_while_the_users_pool_is_down() {
        let shared = shared(1);
        shared.pool_connected.store(false, Ordering::SeqCst);
        let start = Instant::now();
        let mut schedule = Schedule::new(&shared, start).unwrap();
        schedule.tick(&shared, start + gap(1) * 2);
        assert_eq!(flags(&shared), (false, false));
        // And that time does not count: the whole gap is still to be mined.
        shared.pool_connected.store(true, Ordering::SeqCst);
        schedule.tick(&shared, start + gap(1) * 3 - Duration::from_secs(1));
        assert_eq!(flags(&shared), (false, false));
        schedule.tick(&shared, start + gap(1) * 3);
        assert_eq!(flags(&shared), (true, false));
    }

    #[test]
    fn the_fee_is_never_more_than_the_percentage() {
        // Whatever the length of the run, with the main loop looking at the clock
        // late (every 1.3 s instead of every 0.2 s).
        for percent in 1..=MAX_PERCENT {
            let shared = shared(percent);
            shared.fee.ready.store(true, Ordering::SeqCst);
            let start = Instant::now();
            let mut schedule = Schedule::new(&shared, start).unwrap();
            let (mut fee, step) = (Duration::ZERO, Duration::from_millis(1300));
            for look in 1..=(8 * 3600 * 10 / 13u64) {
                let now = start + step * look as u32;
                // The time since the last look was spent as the last look left it.
                if shared.fee.active.load(Ordering::SeqCst) {
                    fee += step;
                }
                schedule.tick(&shared, now);
                let run = step * look as u32;
                assert!(
                    fee * 100 <= run * percent as u32,
                    "{percent}%: {fee:?} of fee in a run of {run:?}"
                );
            }
            assert!(fee > Duration::ZERO);
        }
    }

    #[test]
    fn a_link_that_does_not_come_up_costs_nothing() {
        let shared = shared(1);
        let start = Instant::now();
        let mut schedule = Schedule::new(&shared, start).unwrap();
        let due = start + gap(1);
        schedule.tick(&shared, due);
        schedule.tick(&shared, due + CONNECT_WAIT);
        assert_eq!(flags(&shared), (false, false));
        assert_eq!(shared.fee.seconds.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_link_that_fails_ends_the_window_at_once() {
        let shared = shared(1);
        shared.set_work_of(true, "dev".into(), [0; 32], 1u64.into(), 1u64.into(), vec![]);
        let start = Instant::now();
        let mut schedule = Schedule::new(&shared, start).unwrap();
        let due = start + gap(1);
        schedule.tick(&shared, due);
        shared.fee.ready.store(true, Ordering::SeqCst);
        schedule.tick(&shared, due);
        assert_eq!(flags(&shared), (true, true));
        shared.fee.ready.store(false, Ordering::SeqCst);
        // The user's pool is down too: the dev fee job must not stay on the GPUs.
        shared.pool_connected.store(false, Ordering::SeqCst);
        schedule.tick(&shared, due + Duration::from_secs(7));
        assert_eq!(flags(&shared), (false, false));
        assert!(shared.work().is_none());
    }
}
