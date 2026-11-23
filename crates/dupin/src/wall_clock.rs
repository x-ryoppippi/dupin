use std::time::{Duration, SystemTime};

pub(crate) fn wait_slice(deadline: SystemTime, now: SystemTime) -> Duration {
    // macOS timeout clocks can stop during sleep, so recheck the wall clock after a short wait.
    deadline
        .duration_since(now)
        .unwrap_or_default()
        .min(Duration::from_secs(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_are_bounded_and_wall_clock_jumps_can_pass_the_deadline() {
        let start = SystemTime::UNIX_EPOCH;
        let deadline = start + Duration::from_hours(12);
        assert_eq!(wait_slice(deadline, start), Duration::from_secs(60));
        assert_eq!(
            wait_slice(deadline, deadline - Duration::from_secs(1)),
            Duration::from_secs(1)
        );
        assert_eq!(
            wait_slice(deadline, start + Duration::from_hours(13)),
            Duration::ZERO
        );
    }
}
