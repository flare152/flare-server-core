//! Per-process credential entry bulkhead; owns no business/tenant policy or plaintext.
use std::{collections::HashMap, net::IpAddr, sync::Mutex, time::Duration};
use tokio::{
    sync::{Semaphore, SemaphorePermit, TryAcquireError},
    time::Instant,
};

const MAX_BUCKETS: usize = 4096;
const WINDOW: Duration = Duration::from_secs(1);
#[derive(Hash, PartialEq, Eq)]
enum Subject {
    Ip(IpAddr),
    Key(String),
}
struct Bucket {
    until: Instant,
    count: u32,
}

pub struct CredentialRequestBudget {
    in_flight: Semaphore,
    buckets: Mutex<HashMap<Subject, Bucket>>,
}
impl CredentialRequestBudget {
    pub fn new(max_in_flight: usize) -> Self {
        Self {
            in_flight: Semaphore::new(max_in_flight.min(1024)),
            buckets: Mutex::new(HashMap::new()),
        }
    }
    /// Zero queue. Dropping/cancelling a request releases the slot automatically.
    pub fn try_enter(&self) -> Result<SemaphorePermit<'_>, TryAcquireError> {
        self.in_flight.try_acquire()
    }
    pub fn available_permits(&self) -> usize {
        self.in_flight.available_permits()
    }
    pub fn take_ip(&self, ip: IpAddr) -> bool {
        self.take_at(Subject::Ip(ip), 60, Instant::now())
    }
    /// Call only after authenticating; selector never contains the API Key secret.
    pub fn take_key(&self, selector: &str) -> bool {
        if selector.len() != 12
            || !selector
                .bytes()
                .all(|b| b.is_ascii_uppercase() || matches!(b, b'2'..=b'7'))
        {
            return false;
        }
        self.take_at(Subject::Key(selector.into()), 20, Instant::now())
    }
    fn take_at(&self, subject: Subject, limit: u32, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        buckets.retain(|_, bucket| now < bucket.until);
        if !buckets.contains_key(&subject) && buckets.len() >= MAX_BUCKETS {
            return false;
        }
        let bucket = buckets.entry(subject).or_insert(Bucket {
            until: now + WINDOW,
            count: 0,
        });
        if bucket.count >= limit {
            return false;
        }
        bucket.count += 1;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn memory_bound_never_resets_or_forgets_existing_limit() {
        let budget = CredentialRequestBudget::new(1);
        let now = Instant::now();
        assert!(budget.take_at(Subject::Key("one".into()), 1, now));
        assert!(!budget.take_at(Subject::Key("one".into()), 1, now));
        for index in 1..MAX_BUCKETS {
            assert!(budget.take_at(Subject::Key(index.to_string()), 1, now));
        }
        assert!(!budget.take_at(Subject::Ip("192.0.2.1".parse().unwrap()), 60, now));
        assert!(!budget.take_at(Subject::Key("one".into()), 1, now));
        assert!(budget.take_at(Subject::Key("one".into()), 1, now + WINDOW));
        assert_eq!(budget.buckets.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn zero_queue_and_independent_instances_release_on_drop() {
        let first = CredentialRequestBudget::new(1);
        let second = CredentialRequestBudget::new(1);
        let slot = first.try_enter().unwrap();
        assert!(first.try_enter().is_err());
        assert!(second.try_enter().is_ok());
        drop(slot);
        assert!(first.try_enter().is_ok());
        assert!(CredentialRequestBudget::new(0).try_enter().is_err());
        assert_eq!(CredentialRequestBudget::new(2000).available_permits(), 1024);
    }
    #[tokio::test]
    async fn credentials_and_ips_have_separate_limits_and_reject_secret_keys() {
        let budget = CredentialRequestBudget::new(1);
        let ip = "192.0.2.1".parse().unwrap();
        for _ in 0..60 {
            assert!(budget.take_ip(ip));
        }
        assert!(!budget.take_ip(ip));
        for _ in 0..20 {
            assert!(budget.take_key("AAAAAAAAAAAA"));
        }
        assert!(!budget.take_key("AAAAAAAAAAAA"));
        assert!(budget.take_key("BBBBBBBBBBBB"));
        assert!(!budget.take_key("flr_live_do_not_cache_token"));
        assert!(!budget.take_key("aaaaaaaaaaaa"));
    }
}
