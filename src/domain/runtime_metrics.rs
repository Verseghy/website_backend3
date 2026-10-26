//! The database-pool and job-queue samples of backend3's runtime metrics
//! (ADR 0023 §9, ADR 0016 §3): how raw readings become bounded label sets
//! and values.
//!
//! - The pool's two counts are read one after the other while other tasks
//!   check connections in and out, so they can disagree for a moment;
//!   [`pool_usage`] keeps the pair consistent (`used + idle` is the pool's
//!   size).
//! - The queue's `status` column is free text in Loco's table. The `state`
//!   label takes only the five values Loco writes ([`JOB_STATES`]); any
//!   other value is counted under [`OTHER_JOB_STATE`], so a stray row cannot
//!   create a series of its own (the bounded-label rule of
//!   `http_metrics`).
//! - The age of the oldest queued job counts only jobs that are due: a job
//!   scheduled for later is not waiting yet, while one that is due and still
//!   queued shows that no worker takes jobs. When none is waiting, the age
//!   is 0, so a missing sample always means that the queue could not be
//!   read.

/// The job states of Loco's Postgres queue, as its `status` column spells
/// them (`loco_rs::bgworker::JobStatus`).
pub const JOB_STATES: [&str; 5] = ["queued", "processing", "completed", "failed", "cancelled"];

/// The state whose oldest due job the age measures: jobs waiting for a
/// worker.
pub const QUEUED_JOB_STATE: &str = JOB_STATES[0];

/// The `state` label of jobs whose status is none of [`JOB_STATES`].
pub const OTHER_JOB_STATE: &str = "other";

/// The connections of a database pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolUsage {
    /// Connections checked out by a request, task or job.
    pub used: u64,
    /// Open connections waiting in the pool.
    pub idle: u64,
}

/// Returns the usage of a pool from its two counts.
///
/// - `size`: every open connection of the pool, idle or checked out.
/// - `idle`: the open connections waiting in the pool.
///
/// An `idle` above `size`, which a reading taken between two pool events
/// can give, is capped at `size`.
#[must_use]
pub fn pool_usage(size: u32, idle: usize) -> PoolUsage {
    let size = u64::from(size);
    let idle = u64::try_from(idle).map_or(size, |idle| idle.min(size));
    PoolUsage {
        used: size - idle,
        idle,
    }
}

/// One row of the queue's per-status summary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateCount<'a> {
    /// The `status` column of the rows counted.
    pub state: &'a str,
    /// How many rows have that status.
    pub jobs: i64,
    /// Seconds since the oldest of them that is due became due, or `None`
    /// when none is due.
    pub oldest_due_age_seconds: Option<f64>,
}

/// The queue's sample: jobs by state and the age of the oldest queued job.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueueStats {
    /// Jobs per state, in the order of [`JOB_STATES`].
    pub jobs: [u64; JOB_STATES.len()],
    /// Jobs whose status is none of [`JOB_STATES`].
    pub other_jobs: u64,
    /// Seconds since the oldest due job in [`QUEUED_JOB_STATE`] became due;
    /// 0 when no due job is waiting. Always finite and not negative.
    pub oldest_queued_age_seconds: f64,
}

/// Sums the per-status rows into the queue's sample.
///
/// - `rows`: the summary rows, one per status as the query groups them;
///   repeated statuses add up.
///
/// A negative count is taken as 0 and sums saturate, so a corrupt reading
/// cannot wrap. Only [`QUEUED_JOB_STATE`] rows set the age; their oldest
/// finite, non-negative age wins, and any other age is ignored.
#[must_use]
pub fn queue_stats<'a>(rows: impl IntoIterator<Item = StateCount<'a>>) -> QueueStats {
    let mut stats = QueueStats::default();
    for row in rows {
        let jobs = u64::try_from(row.jobs).unwrap_or(0);
        let slot = match JOB_STATES.iter().position(|state| *state == row.state) {
            Some(index) => &mut stats.jobs[index],
            None => &mut stats.other_jobs,
        };
        *slot = slot.saturating_add(jobs);
        if row.state == QUEUED_JOB_STATE
            && let Some(age) = row.oldest_due_age_seconds
            && age.is_finite()
            && age > stats.oldest_queued_age_seconds
        {
            stats.oldest_queued_age_seconds = age;
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn row(state: &str, jobs: i64, age: Option<f64>) -> StateCount<'_> {
        StateCount {
            state,
            jobs,
            oldest_due_age_seconds: age,
        }
    }

    #[test]
    fn used_is_the_rest_of_the_pool() {
        assert_eq!(pool_usage(5, 2), PoolUsage { used: 3, idle: 2 });
        assert_eq!(pool_usage(0, 0), PoolUsage { used: 0, idle: 0 });
        assert_eq!(pool_usage(1, 1), PoolUsage { used: 0, idle: 1 });
    }

    #[test]
    fn idle_above_the_size_is_capped() {
        assert_eq!(pool_usage(2, 3), PoolUsage { used: 0, idle: 2 });
    }

    #[test]
    fn jobs_are_counted_by_state() {
        let stats = queue_stats([
            row("queued", 4, Some(12.5)),
            row("completed", 30, None),
            row("failed", 2, None),
        ]);
        assert_eq!(stats.jobs, [4, 0, 30, 2, 0]);
        assert_eq!(stats.other_jobs, 0);
        assert!((stats.oldest_queued_age_seconds - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn an_empty_queue_has_zero_jobs_and_age() {
        assert_eq!(queue_stats([]), QueueStats::default());
        // Queued jobs that are all scheduled for later are not waiting.
        let stats = queue_stats([row("queued", 3, None)]);
        assert_eq!(stats.jobs[0], 3);
        assert!(stats.oldest_queued_age_seconds.abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_states_share_one_label() {
        let stats = queue_stats([row("Queued", 1, Some(9.0)), row("", 2, None)]);
        assert_eq!(stats.jobs, [0; 5]);
        assert_eq!(stats.other_jobs, 3);
        // Only the exact `queued` state sets the age.
        assert!(stats.oldest_queued_age_seconds.abs() < f64::EPSILON);
    }

    #[test]
    fn ages_of_other_states_and_broken_ages_are_ignored() {
        let stats = queue_stats([
            row("processing", 1, Some(500.0)),
            row("queued", 1, Some(f64::NAN)),
            row("queued", 1, Some(f64::INFINITY)),
            row("queued", 1, Some(-3.0)),
        ]);
        assert_eq!(stats.jobs[0], 3);
        assert!(stats.oldest_queued_age_seconds.abs() < f64::EPSILON);
    }

    #[test]
    fn negative_counts_are_zero_and_sums_saturate() {
        let stats = queue_stats([
            row("failed", -5, None),
            row("cancelled", i64::MAX, None),
            row("cancelled", i64::MAX, None),
            row("cancelled", i64::MAX, None),
        ]);
        assert_eq!(stats.jobs[3], 0);
        assert_eq!(stats.jobs[4], u64::MAX);
    }

    #[test]
    fn the_states_are_distinct_and_other_is_not_one_of_them() {
        for (index, state) in JOB_STATES.iter().enumerate() {
            assert!(!JOB_STATES[index + 1..].contains(state), "{state}");
        }
        assert!(!JOB_STATES.contains(&OTHER_JOB_STATE));
        assert_eq!(QUEUED_JOB_STATE, "queued");
    }

    /// A status that is one of Loco's, or an arbitrary string.
    fn any_state() -> impl Strategy<Value = String> {
        prop_oneof![
            proptest::sample::select(JOB_STATES.to_vec()).prop_map(str::to_owned),
            any::<String>(),
        ]
    }

    proptest! {
        #[test]
        fn the_pool_counts_always_add_up_to_its_size(size in any::<u32>(), idle in any::<usize>()) {
            let usage = pool_usage(size, idle);
            prop_assert_eq!(usage.used + usage.idle, u64::from(size));
            prop_assert!(usage.idle <= u64::try_from(idle).unwrap_or(u64::MAX));
        }

        #[test]
        fn every_job_is_counted_once_and_the_age_is_the_oldest_waiting_one(
            rows in proptest::collection::vec(
                (any_state(), -10i64..1_000_000_000_000, proptest::option::of(any::<f64>())),
                0..12,
            ),
        ) {
            let stats = queue_stats(
                rows.iter().map(|(state, jobs, age)| row(state, *jobs, *age)),
            );
            let counted: u64 = rows.iter().map(|(_, jobs, _)| u64::try_from(*jobs).unwrap_or(0)).sum();
            prop_assert_eq!(stats.jobs.iter().sum::<u64>() + stats.other_jobs, counted);
            let expected_age = rows
                .iter()
                .filter(|(state, _, _)| state == QUEUED_JOB_STATE)
                .filter_map(|(_, _, age)| *age)
                .filter(|age| age.is_finite() && *age >= 0.0)
                .fold(0.0, f64::max);
            prop_assert!(stats.oldest_queued_age_seconds.is_finite());
            prop_assert!(stats.oldest_queued_age_seconds >= 0.0);
            // `+ 0.0` turns a -0.0 that `f64::max` may keep into 0.0.
            prop_assert_eq!(
                stats.oldest_queued_age_seconds.to_bits(),
                (expected_age + 0.0).to_bits()
            );
        }
    }
}
