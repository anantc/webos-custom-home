use std::time::Duration;
use tokio::time::Instant;

struct Launch {
    number: u64,
    detected: Instant,
    initial_snapshot: bool,
    acknowledged: bool,
    foreground: bool,
}

pub(super) struct Timing {
    connection: u64,
    sequence: u64,
    launch: Option<Launch>,
}

impl Timing {
    pub(super) fn new(connection: u64) -> Self {
        Self {
            connection,
            sequence: 0,
            launch: None,
        }
    }

    pub(super) fn event(&self, details: &str) -> String {
        format!("Watch timing: connection={} {details}", self.connection)
    }

    pub(super) fn begin_launch(&mut self, detected: Instant, initial_snapshot: bool) {
        self.sequence += 1;
        self.launch = Some(Launch {
            number: self.sequence,
            detected,
            initial_snapshot,
            acknowledged: false,
            foreground: false,
        });
    }

    pub(super) fn sent(&self, now: Instant) -> String {
        let launch = self
            .launch
            .as_ref()
            .expect("launch was recorded before sending");
        self.event(&format!(
            "event=launch_sent launch={} initial_snapshot={} dispatch_ms={}",
            launch.number,
            launch.initial_snapshot,
            now.duration_since(launch.detected).as_millis()
        ))
    }

    pub(super) fn acknowledged(&mut self, now: Instant) -> Option<String> {
        self.milestone(now, false)
    }

    pub(super) fn foreground(&mut self, now: Instant) -> Option<String> {
        self.milestone(now, true)
    }

    fn milestone(&mut self, now: Instant, foreground: bool) -> Option<String> {
        let launch = self.launch.as_mut()?;
        let seen = if foreground {
            &mut launch.foreground
        } else {
            &mut launch.acknowledged
        };
        if *seen {
            return None;
        }
        *seen = true;
        let event = if foreground {
            "home_foreground"
        } else {
            "launch_ack"
        };
        let details = format!(
            "event={event} launch={} since_detection_ms={}",
            launch.number,
            now.duration_since(launch.detected).as_millis()
        );
        if launch.foreground && launch.acknowledged {
            self.launch = None;
        }
        Some(self.event(&details))
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.launch
            .as_ref()
            .map(|l| l.detected + Duration::from_secs(10))
    }

    pub(super) fn expire(&mut self, now: Instant) -> Option<String> {
        if self.deadline().is_some_and(|deadline| now >= deadline) {
            self.end("observation_timeout", now)
        } else {
            None
        }
    }

    pub(super) fn end(&mut self, reason: &str, now: Instant) -> Option<String> {
        let launch = self.launch.take()?;
        Some(self.event(&format!(
            "event=launch_incomplete launch={} reason={reason} since_detection_ms={} ack_received={} foreground_received={}",
            launch.number, now.duration_since(launch.detected).as_millis(), launch.acknowledged, launch.foreground
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_and_foreground_are_distinct_and_can_arrive_in_either_order() {
        for foreground_first in [false, true] {
            let mut timing = Timing::new(2);
            let start = Instant::now();
            timing.begin_launch(start, false);
            assert!(timing
                .sent(start + Duration::from_millis(4))
                .contains("dispatch_ms=4"));
            let first = start + Duration::from_millis(20);
            let second = start + Duration::from_millis(300);
            let (a, b) = if foreground_first {
                let a = timing.foreground(first).unwrap();
                assert!(timing.foreground(first).is_none());
                assert!(timing.deadline().is_some());
                (a, timing.acknowledged(second).unwrap())
            } else {
                let a = timing.acknowledged(first).unwrap();
                assert!(timing.acknowledged(first).is_none());
                assert!(timing.deadline().is_some());
                (a, timing.foreground(second).unwrap())
            };
            assert!(a.contains("since_detection_ms=20"));
            assert!(b.contains("since_detection_ms=300"));
            assert!(a.contains("connection=2"));
            assert!(a.contains("launch=1"));
            assert!(timing.deadline().is_none());
            assert!(timing.foreground(second).is_none());
        }
    }

    #[test]
    fn missing_foreground_is_not_reported_as_a_completed_launch() {
        let mut timing = Timing::new(1);
        let start = Instant::now();
        timing.begin_launch(start, true);
        assert!(timing.sent(start).contains("initial_snapshot=true"));
        timing.acknowledged(start + Duration::from_millis(10));
        assert!(timing.expire(start + Duration::from_secs(9)).is_none());
        let line = timing.expire(start + Duration::from_secs(10)).unwrap();
        assert!(line.contains("reason=observation_timeout"));
        assert!(line.contains("ack_received=true foreground_received=false"));
        assert!(timing.foreground(start + Duration::from_secs(11)).is_none());
        timing.begin_launch(start + Duration::from_secs(12), false);
        assert!(timing
            .sent(start + Duration::from_secs(12))
            .contains("launch=2"));
    }

    #[test]
    fn disconnect_reports_the_missing_stage_without_reusing_old_timings() {
        let mut timing = Timing::new(3);
        let start = Instant::now();
        timing.begin_launch(start, false);
        timing.foreground(start + Duration::from_millis(30));
        let line = timing
            .end("connection_ended", start + Duration::from_secs(1))
            .unwrap();
        assert!(line.contains("ack_received=false foreground_received=true"));
        assert!(timing
            .acknowledged(start + Duration::from_secs(2))
            .is_none());
    }
}
