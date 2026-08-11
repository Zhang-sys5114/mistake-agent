//! 时钟抽象（可测试性）。

use super::*;

// ---------- 时钟（可测试性） ----------

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone)]
pub struct FakeClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }
    pub fn advance(&self, d: Duration) {
        *self.now.lock().expect("clock poisoned") += chrono::Duration::from_std(d).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock poisoned")
    }
}
