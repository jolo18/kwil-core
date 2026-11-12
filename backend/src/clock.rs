use chrono::{DateTime, NaiveDate, Utc};
use std::sync::Mutex;

/// Source of "now". Tests use [`FixedClock`] so scheduled work can be triggered on demand.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    fn today(&self) -> NaiveDate {
        self.now().date_naive()
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
}

impl FixedClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self { now: Mutex::new(now) }
    }
    pub fn set(&self, now: DateTime<Utc>) {
        *self.now.lock().unwrap() = now;
    }
    pub fn advance_days(&self, days: i64) {
        let mut now = self.now.lock().unwrap();
        *now += chrono::Duration::days(days);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }
}
