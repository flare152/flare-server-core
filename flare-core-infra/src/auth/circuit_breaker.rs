//! Business-neutral, per-upstream consecutive-failure circuit breaker.
//!
//! Closed: every request is forwarded; `threshold` consecutive upstream
//! failures open the breaker. Open: requests fail locally until the window
//! ends. Half-open: exactly one probe reaches the upstream; its success closes
//! the breaker and its failure reopens it for another window. Other requests
//! keep failing locally while that probe is outstanding.
//!
//! The window depends on the failure that decides it. An upstream that
//! answered badly or too late (5xx, a timeout after the request reached it) is
//! spared for the full `open_for`. An upstream that could not be reached at all
//! (connect refused, unreachable, connect timeout) gets no load from waiting,
//! so the next probe comes after the short `reconnect_after` instead: once it
//! is reachable again, the breaker closes within about that window plus one
//! connect timeout, instead of up to a full window more (W8-S1: Control back
//! after a restart, Management still refusing for 10 s windows each preceded
//! by a 2 s connect timeout).

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Retry hint while another request is already probing the upstream.
const PROBE_RETRY_AFTER: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The upstream answered below 500.
    Success,
    /// An upstream-attributable timeout after the request reached it, a broken
    /// exchange or a 5xx response.
    Failure,
    /// No connection to the upstream: refused, unreachable or the connect
    /// timed out. Counted as a failure; a window it decides is the short one.
    Unreachable,
    /// Says nothing about upstream health: the caller's own body failed, or
    /// the caller went away before the upstream answered.
    Neutral,
}

pub struct CircuitBreaker {
    partition: &'static str,
    threshold: u32,
    open_for: Duration,
    reconnect_after: Duration,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    consecutive_failures: u32,
    /// `Some` while open; once the instant has passed the breaker is half-open.
    open_until: Option<Instant>,
    probing: bool,
    /// Probes that failed since the breaker opened.
    failed_probes: u32,
}

/// One admitted request. Dropping it unsettled counts as [`Outcome::Neutral`],
/// so an abandoned probe cannot wedge the breaker half-open.
pub struct Ticket<'a> {
    breaker: &'a CircuitBreaker,
    probe: bool,
    settled: bool,
}

impl CircuitBreaker {
    /// Static partition labels only; `reconnect_after` is capped at `open_for`.
    pub fn new(
        partition: &'static str,
        threshold: u32,
        open_for: Duration,
        reconnect_after: Duration,
    ) -> Self {
        Self {
            partition,
            threshold: threshold.max(1),
            open_for,
            reconnect_after: reconnect_after.min(open_for),
            state: Mutex::new(State::default()),
        }
    }

    /// How long a failure of this kind keeps the breaker open.
    fn window_for(&self, outcome: Outcome) -> Duration {
        if outcome == Outcome::Unreachable {
            self.reconnect_after
        } else {
            self.open_for
        }
    }

    /// `Err` carries how long the caller should wait before retrying.
    pub fn try_admit(&self) -> Result<Ticket<'_>, Duration> {
        self.try_admit_at(Instant::now())
    }

    fn try_admit_at(&self, now: Instant) -> Result<Ticket<'_>, Duration> {
        let mut state = self.lock();
        let probe = match state.open_until {
            None => false,
            Some(until) if now < until => return Err(until - now),
            Some(_) if state.probing => return Err(PROBE_RETRY_AFTER),
            Some(_) => {
                state.probing = true;
                true
            }
        };
        Ok(Ticket {
            breaker: self,
            probe,
            settled: false,
        })
    }

    fn record(&self, probe: bool, outcome: Outcome, now: Instant) {
        let mut state = self.lock();
        if probe {
            state.probing = false;
        }
        match (outcome, probe, state.open_until.is_some()) {
            (Outcome::Neutral, _, _) => {}
            // A request admitted before the breaker opened neither closes nor
            // extends it; only the half-open probe decides.
            (_, false, true) => {}
            (Outcome::Success, _, _) => {
                if state.open_until.take().is_some() {
                    tracing::info!(
                        failed_probes = state.failed_probes,
                        partition = self.partition,
                        "gateway upstream circuit closed after a successful probe"
                    );
                }
                state.consecutive_failures = 0;
                state.failed_probes = 0;
            }
            (Outcome::Failure | Outcome::Unreachable, true, _) => {
                let window = self.window_for(outcome);
                state.open_until = Some(now + window);
                state.failed_probes = state.failed_probes.saturating_add(1);
                // Short windows probe an unreachable upstream every few
                // seconds: log the 1st, 2nd, 4th, 8th… failed probe only.
                if state.failed_probes.is_power_of_two() {
                    tracing::warn!(
                        open_ms = window.as_millis() as u64,
                        failed_probes = state.failed_probes,
                        unreachable = outcome == Outcome::Unreachable,
                        partition = self.partition,
                        "gateway upstream probe failed; circuit reopened"
                    );
                }
            }
            (Outcome::Failure | Outcome::Unreachable, false, false) => {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                if state.consecutive_failures >= self.threshold {
                    let window = self.window_for(outcome);
                    state.consecutive_failures = 0;
                    state.failed_probes = 0;
                    state.open_until = Some(now + window);
                    tracing::warn!(
                        failures = self.threshold,
                        open_ms = window.as_millis() as u64,
                        unreachable = outcome == Outcome::Unreachable,
                        partition = self.partition,
                        "gateway upstream circuit opened"
                    );
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // The state is a few plain fields updated atomically under the lock;
        // a panic elsewhere cannot leave it half-written.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Ticket<'_> {
    pub fn settle(self, outcome: Outcome) {
        self.settle_at(outcome, Instant::now());
    }

    fn settle_at(mut self, outcome: Outcome, now: Instant) {
        self.settled = true;
        self.breaker.record(self.probe, outcome, now);
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        if !self.settled && self.probe {
            self.breaker.record(true, Outcome::Neutral, Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(10);

    fn fail(breaker: &CircuitBreaker, now: Instant) {
        breaker
            .try_admit_at(now)
            .expect("closed breaker admits")
            .settle_at(Outcome::Failure, now);
    }

    #[test]
    fn opens_only_after_consecutive_failures() {
        let breaker = CircuitBreaker::new("test", 3, WINDOW, WINDOW);
        let now = Instant::now();
        fail(&breaker, now);
        fail(&breaker, now);
        // A success in between resets the streak.
        breaker
            .try_admit_at(now)
            .unwrap()
            .settle_at(Outcome::Success, now);
        fail(&breaker, now);
        fail(&breaker, now);
        assert!(
            breaker.try_admit_at(now).is_ok(),
            "two failures stay closed"
        );
        fail(&breaker, now);
        let retry = breaker
            .try_admit_at(now)
            .err()
            .expect("third failure opens");
        assert_eq!(retry, WINDOW);
        let later = now + Duration::from_secs(4);
        assert_eq!(
            breaker.try_admit_at(later).err(),
            Some(Duration::from_secs(6))
        );
    }

    #[test]
    fn neutral_outcomes_never_count() {
        let breaker = CircuitBreaker::new("test", 1, WINDOW, WINDOW);
        let now = Instant::now();
        for _ in 0..5 {
            breaker
                .try_admit_at(now)
                .unwrap()
                .settle_at(Outcome::Neutral, now);
        }
        assert!(breaker.try_admit_at(now).is_ok());
    }

    #[test]
    fn half_open_admits_one_probe_and_its_success_closes() {
        let breaker = CircuitBreaker::new("test", 1, WINDOW, WINDOW);
        let now = Instant::now();
        fail(&breaker, now);
        let expired = now + WINDOW;
        let probe = breaker.try_admit_at(expired).expect("window elapsed");
        assert_eq!(
            breaker.try_admit_at(expired).err(),
            Some(PROBE_RETRY_AFTER),
            "only one probe while half-open"
        );
        probe.settle_at(Outcome::Success, expired);
        for _ in 0..3 {
            breaker
                .try_admit_at(expired)
                .expect("closed after probe success")
                .settle_at(Outcome::Success, expired);
        }
    }

    #[test]
    fn failed_probe_reopens_for_a_full_window() {
        let breaker = CircuitBreaker::new("test", 2, WINDOW, WINDOW);
        let now = Instant::now();
        fail(&breaker, now);
        fail(&breaker, now);
        let expired = now + WINDOW;
        breaker
            .try_admit_at(expired)
            .unwrap()
            .settle_at(Outcome::Failure, expired);
        assert_eq!(breaker.try_admit_at(expired).err(), Some(WINDOW));
        // After the second window a single probe is admitted again.
        assert!(breaker.try_admit_at(expired + WINDOW).is_ok());
    }

    #[test]
    fn abandoned_probe_releases_the_half_open_slot() {
        let breaker = CircuitBreaker::new("test", 1, WINDOW, WINDOW);
        let now = Instant::now();
        fail(&breaker, now);
        let expired = now + WINDOW;
        drop(breaker.try_admit_at(expired).unwrap());
        let probe = breaker
            .try_admit_at(expired)
            .expect("a cancelled probe must not wedge half-open");
        probe.settle_at(Outcome::Success, expired);
        assert!(breaker.try_admit_at(expired).is_ok());
    }

    fn settle(breaker: &CircuitBreaker, outcome: Outcome, now: Instant) {
        breaker
            .try_admit_at(now)
            .expect("admitted")
            .settle_at(outcome, now);
    }

    #[test]
    fn an_unreachable_upstream_is_probed_again_after_the_short_window() {
        let reconnect = Duration::from_secs(2);
        let breaker = CircuitBreaker::new("test", 2, WINDOW, reconnect);
        let now = Instant::now();
        // Unreachable counts as a failure; the trip it decides is short.
        settle(&breaker, Outcome::Unreachable, now);
        settle(&breaker, Outcome::Unreachable, now);
        assert_eq!(breaker.try_admit_at(now).err(), Some(reconnect));
        // A probe that cannot connect either: short again, however long it lasts.
        let mut at = now + reconnect;
        for _ in 0..5 {
            settle(&breaker, Outcome::Unreachable, at);
            assert_eq!(breaker.try_admit_at(at).err(), Some(reconnect));
            at += reconnect;
        }
        // Reached but failing (5xx, timeout): spared for the full window.
        settle(&breaker, Outcome::Failure, at);
        assert_eq!(breaker.try_admit_at(at).err(), Some(WINDOW));
        at += WINDOW;
        settle(&breaker, Outcome::Success, at);
        assert!(breaker.try_admit_at(at).is_ok(), "closed");
        // A trip decided by a 5xx keeps the full window too.
        settle(&breaker, Outcome::Unreachable, at);
        settle(&breaker, Outcome::Failure, at);
        assert_eq!(breaker.try_admit_at(at).err(), Some(WINDOW));
        // The short window never exceeds the full one.
        let capped = CircuitBreaker::new("test", 1, Duration::from_secs(1), Duration::from_secs(5));
        settle(&capped, Outcome::Unreachable, now);
        assert_eq!(capped.try_admit_at(now).err(), Some(Duration::from_secs(1)));
    }

    /// When Management first gets through after Control became reachable
    /// again at `healed` (seconds after it started): a request every second,
    /// each connect that cannot reach Control failing after 2 s, the default
    /// threshold of 5 and full window of 10 s.
    fn first_success_after(healed: u64, reconnect: Duration) -> Duration {
        let breaker = CircuitBreaker::new("test", 5, Duration::from_secs(10), reconnect);
        let start = Instant::now();
        let healed = start + Duration::from_secs(healed);
        let step = Duration::from_millis(100);
        let mut in_flight: Vec<(Instant, Ticket<'_>, Outcome)> = Vec::new();
        let mut now = start;
        loop {
            let (due, pending): (Vec<_>, Vec<_>) =
                in_flight.into_iter().partition(|(at, _, _)| *at <= now);
            in_flight = pending;
            for (at, ticket, outcome) in due {
                if outcome == Outcome::Success {
                    return at - healed;
                }
                ticket.settle_at(outcome, at);
            }
            if (now - start).as_millis().is_multiple_of(1000)
                && let Ok(ticket) = breaker.try_admit_at(now)
            {
                in_flight.push(if now >= healed {
                    (now + Duration::from_millis(10), ticket, Outcome::Success)
                } else {
                    (now + Duration::from_secs(2), ticket, Outcome::Unreachable)
                });
            }
            now += step;
            assert!(now < start + Duration::from_secs(600), "never recovered");
        }
    }

    #[test]
    fn recovery_after_an_unreachable_spell_is_bounded_by_the_short_window() {
        let reconnect = Duration::from_secs(2);
        let mut worst_full = Duration::ZERO;
        for healed in 1..=60 {
            let short = first_success_after(healed, reconnect);
            // At most: the rest of a short window, a probe's connect timeout
            // that began just before Control healed, and the next request.
            assert!(
                short <= reconnect + Duration::from_secs(2) + Duration::from_secs(1),
                "healed at {healed} s: first success {short:?} later"
            );
            worst_full = worst_full.max(first_success_after(healed, Duration::from_secs(10)));
        }
        // With every window the full 10 s (the behaviour before), it took up
        // to a window and a connect timeout more.
        assert!(worst_full >= Duration::from_secs(11), "{worst_full:?}");
    }

    #[test]
    fn stragglers_admitted_while_closed_cannot_close_or_extend() {
        let breaker = CircuitBreaker::new("test", 1, WINDOW, WINDOW);
        let now = Instant::now();
        let straggler_ok = breaker.try_admit_at(now).unwrap();
        let straggler_err = breaker.try_admit_at(now).unwrap();
        fail(&breaker, now);
        let later = now + Duration::from_secs(9);
        straggler_ok.settle_at(Outcome::Success, later);
        assert_eq!(
            breaker.try_admit_at(later).err(),
            Some(Duration::from_secs(1)),
            "success from before the trip does not close the breaker"
        );
        straggler_err.settle_at(Outcome::Failure, later);
        assert!(
            breaker.try_admit_at(now + WINDOW).is_ok(),
            "failure from before the trip does not extend the window"
        );
    }
}
