use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

#[derive(Default)]
pub(super) struct Limits {
    devices: BTreeMap<String, Bucket>,
    sources: BTreeMap<String, Bucket>,
}
struct Bucket {
    attempts: VecDeque<Duration>,
    tokens: f64,
    updated: Duration,
}
impl Bucket {
    fn check(&mut self, now: Duration, limit: u32) -> Result<(), u32> {
        let limit = limit.max(1);
        self.tokens = (self.tokens
            + now.saturating_sub(self.updated).as_secs_f64() * f64::from(limit) / 60.0)
            .min(3.0);
        self.updated = now;
        while self
            .attempts
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= Duration::from_mins(1))
        {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= usize::try_from(limit).unwrap_or(usize::MAX) {
            return Err(self
                .attempts
                .front()
                .map_or(60, |t| {
                    60_u32.saturating_sub(
                        u32::try_from(now.saturating_sub(*t).as_secs()).unwrap_or(60),
                    )
                })
                .max(1));
        }
        if self.tokens < 1.0 {
            return Err(60_u32.div_ceil(limit).max(1));
        }
        self.tokens -= 1.0;
        self.attempts.push_back(now);
        Ok(())
    }
}
impl Limits {
    pub(super) fn check(
        &mut self,
        key: &str,
        source: bool,
        now: Duration,
        limit: u32,
    ) -> Result<(), u32> {
        let buckets = if source {
            &mut self.sources
        } else {
            &mut self.devices
        };
        buckets.retain(|_, b| now.saturating_sub(b.updated) < Duration::from_mins(1));
        if buckets.len() >= 4096 && !buckets.contains_key(key) {
            return Err(60);
        }
        buckets
            .entry(key.to_owned())
            .or_insert_with(|| Bucket {
                attempts: VecDeque::new(),
                tokens: 3.0,
                updated: now,
            })
            .check(now, limit)
    }
}
