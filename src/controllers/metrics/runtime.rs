//! The runtime families of `/metrics`: the database pool, Loco's Postgres
//! job queue and the process (ADR 0023 §9, ADR 0016 §3).
//!
//! The HTTP families change as requests happen; these are read when the hub
//! scrapes. [`RuntimeMetrics::refresh`] samples every source, and a
//! collector registered on the application's registry writes the latest
//! sample when the registry is encoded. Sampling is `async` (the queue is a
//! database query) and `prometheus_client` encodes synchronously, so the
//! two meet in one shared sample. Of two scrapes at the same moment, one
//! may write the sample the other refreshed, which is just as fresh.
//!
//! A source that cannot be read leaves its families out of that scrape; it
//! never fails the scrape, which would hide the families that did work. So
//! a missing family always means "could not be read", never zero:
//!
//! - `db_client_connections{state="used"|"idle"}`: the application's
//!   Sea-ORM pool (not the queue's own pool), read before the queue query so
//!   that the scrape's own connection is not counted.
//! - `loco_queue_jobs{state}` and
//!   `loco_queue_oldest_queued_job_age_seconds`: one query of
//!   `pg_loco_queue` ([`crate::models::job_queue`]). The age grows while no
//!   worker takes jobs, for example in a process started without
//!   `--server-and-worker` (ADR 0016, "What must be true in code and
//!   tests").
//! - `process_cpu_seconds_total` and `process_resident_memory_bytes`: the
//!   standard Prometheus process metrics, from `/proc/self`, parsed by
//!   [`crate::domain::proc_stat`]. On a system without procfs they are
//!   absent.
//!
//! The queue query uses a connection of the application's pool, which the
//! requests need too, so the scrape bounds it three ways: PostgreSQL cancels
//! it after [`QUEUE_STATEMENT_TIMEOUT`], the scrape stops waiting after
//! [`QUEUE_QUERY_TIMEOUT`], and only one queue read runs at a time. Scrapes
//! of a locked or overloaded database therefore do not pile up connections
//! that the requests are waiting for.

use std::{
    fmt, io,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use prometheus_client::{
    collector::Collector,
    encoding::{DescriptorEncoder, EncodeMetric},
    metrics::{MetricType, counter::ConstCounter, gauge::ConstGauge},
    registry::{Registry, Unit},
};
use sea_orm::{DatabaseConnection, DatabaseConnectionType};

use crate::{
    domain::{
        proc_stat::{self, ProcParseError},
        runtime_metrics::{
            JOB_STATES, OTHER_JOB_STATE, PoolUsage, QueueStats, StateCount, pool_usage, queue_stats,
        },
    },
    models::job_queue,
};

/// How long a scrape waits for the queue query, connection checkout
/// included. Prometheus' default scrape timeout is 10 s, so a slow database
/// delays the scrape at most this long and the other families still arrive.
/// The query is an aggregate over a table from which `jobs tidy` removes
/// the completed and cancelled jobs every night (ADR 0016 §4), so it
/// normally takes milliseconds.
pub const QUEUE_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

/// How long PostgreSQL lets the queue query run before it cancels it
/// (`statement_timeout`). Shorter than [`QUEUE_QUERY_TIMEOUT`], so that the
/// server normally ends a slow query before the scrape gives up on it: a
/// query the scrape merely drops keeps its connection checked out until the
/// server answers ([`job_queue::jobs_by_status`]).
pub const QUEUE_STATEMENT_TIMEOUT: Duration = Duration::from_millis(1500);

// The ordering above, checked when the crate compiles.
const _: () = assert!(QUEUE_STATEMENT_TIMEOUT.as_millis() < QUEUE_QUERY_TIMEOUT.as_millis());

/// The file that [`proc_stat::cpu_seconds`] parses.
const STAT_PATH: &str = "/proc/self/stat";

/// The file that [`proc_stat::resident_bytes`] parses.
const STATUS_PATH: &str = "/proc/self/status";

/// The name and help of the pool family. Collectors write the help as
/// given, so it carries the closing period that `Registry::register` adds
/// to the others.
///
/// A name of backend3's own, not an OpenTelemetry one. Translated to
/// Prometheus, the current semantic convention, `db.client.connection.count`
/// (still in development), would end in `_count`, which `promtool check
/// metrics` rejects for a gauge, because it is a suffix of histograms and
/// summaries. Asked to choose between this name and the OpenTelemetry
/// spellings, the maintainer answered "Confirm as built" on 2026-10-25, for
/// this name, the other names it asked about and the metrics port
/// (research: DECISIONS R15, D85).
const DB_CONNECTIONS: (&str, &str) = (
    "db_client_connections",
    "Connections of the application's database pool, by state: used (checked out) or idle.",
);

/// The name and help of the jobs family.
const QUEUE_JOBS: (&str, &str) = (
    "loco_queue_jobs",
    "Jobs in the Postgres job queue table pg_loco_queue, by state.",
);

/// The name (before the unit) and help of the queue-age family.
const QUEUE_OLDEST_AGE: (&str, &str) = (
    "loco_queue_oldest_queued_job_age",
    "Time since the oldest queued job became due, 0 when none is waiting; it grows while no \
     worker takes jobs.",
);

/// The name (before the unit) and help of the CPU family.
const PROCESS_CPU: (&str, &str) = (
    "process_cpu",
    "Total user and system CPU time spent in seconds.",
);

/// The name (before the unit) and help of the memory family.
const PROCESS_RESIDENT_MEMORY: (&str, &str) =
    ("process_resident_memory", "Resident memory size in bytes.");

/// One reading of every runtime source; `None` where a source could not be
/// read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RuntimeSample {
    pool: Option<PoolUsage>,
    queue: Option<QueueStats>,
    cpu_seconds: Option<f64>,
    resident_bytes: Option<u64>,
}

/// The latest sample, shared by [`RuntimeMetrics`] and its collector.
type SharedSample = Arc<Mutex<RuntimeSample>>;

/// Reads the shared sample. A panic while the lock was held cannot leave a
/// half-written sample (it is replaced whole), so a poisoned lock is used
/// as it is.
fn latest(shared: &SharedSample) -> RuntimeSample {
    *shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The sources of the runtime families, and the sample they last gave.
#[derive(Clone, Debug)]
pub struct RuntimeMetrics {
    db: DatabaseConnection,
    sample: SharedSample,
    /// Held while a queue read runs. A tokio mutex, because its guard is
    /// held across the query's `.await`.
    queue_read: Arc<tokio::sync::Mutex<()>>,
}

impl RuntimeMetrics {
    /// Registers the runtime families on `registry` and returns the handle
    /// that refreshes them.
    ///
    /// - `registry`: the application's registry, still being built in
    ///   `App::after_context`.
    /// - `db`: the application's database connection (`ctx.db`): its pool
    ///   is the one measured, and the queue table is read through it.
    #[must_use]
    pub fn register(registry: &mut Registry, db: DatabaseConnection) -> Self {
        let sample = SharedSample::default();
        registry.register_collector(Box::new(RuntimeCollector {
            sample: Arc::clone(&sample),
        }));
        Self {
            db,
            sample,
            queue_read: Arc::default(),
        }
    }

    /// Reads every source once and keeps the result for the next encoding
    /// of the registry. It never fails: a source that cannot be read is
    /// logged and left out until a later refresh reads it.
    ///
    /// The queue read is bounded as the module documentation says. A
    /// refresh that finds another one's queue read still running leaves the
    /// queue families out instead of taking a second connection.
    pub async fn refresh(&self) {
        // Before the query, which checks a connection out itself.
        let pool = read_pool(&self.db);
        let queue = self.read_queue().await;
        let sample = RuntimeSample {
            pool,
            queue,
            cpu_seconds: read_proc(STAT_PATH, proc_stat::cpu_seconds),
            resident_bytes: read_proc(STATUS_PATH, proc_stat::resident_bytes),
        };
        *self.sample.lock().unwrap_or_else(PoisonError::into_inner) = sample;
    }

    /// The queue's sample, or `None` when it cannot be read in time or a
    /// read is already running.
    async fn read_queue(&self) -> Option<QueueStats> {
        // The guard lives until the read ends. `try_lock`, not `lock`:
        // waiting would only queue more scrapes behind a slow database. A
        // healthy read takes milliseconds, so two scrapes rarely meet here.
        let Ok(_running) = self.queue_read.try_lock() else {
            tracing::debug!(
                "a job-queue read is still running; the queue families are left out of this scrape"
            );
            return None;
        };
        let rows = bounded(
            QUEUE_QUERY_TIMEOUT,
            job_queue::jobs_by_status(&self.db, QUEUE_STATEMENT_TIMEOUT),
            "the job queue",
        )
        .await?;
        Some(queue_stats(rows.iter().map(|row| StateCount {
            state: &row.status,
            jobs: row.jobs,
            oldest_due_age_seconds: row.oldest_due_age_seconds,
        })))
    }
}

/// The pool counts of `db`, or `None` when it is not a PostgreSQL pool (a
/// disconnected or mock connection).
fn read_pool(db: &DatabaseConnection) -> Option<PoolUsage> {
    // `get_postgres_connection_pool` panics on any other kind of connection,
    // hence the check. Which other kinds exist depends on Sea-ORM's enabled
    // features, so they are not named.
    matches!(
        db.inner,
        DatabaseConnectionType::SqlxPostgresPoolConnection(_)
    )
    .then(|| {
        let pool = db.get_postgres_connection_pool();
        pool_usage(pool.size(), pool.num_idle())
    })
}

/// Awaits `read` for at most `limit`; `None`, with a warning that names
/// `source`, when it fails or takes longer.
///
/// On a timeout the query's future is dropped. sqlx then reads the rest of
/// the reply in the background and tests the connection before it goes back
/// to the pool, closing it if that fails (sqlx-core 0.9,
/// `PoolConnection::return_to_pool`), so a half-read reply never reaches the
/// next user of that connection. Until the server has answered, the
/// connection stays checked out: this timeout frees the scrape, not the
/// connection, which is why the queue query also has a server-side limit
/// ([`QUEUE_STATEMENT_TIMEOUT`]).
async fn bounded<T, E: fmt::Display>(
    limit: Duration,
    read: impl Future<Output = Result<T, E>>,
    source: &str,
) -> Option<T> {
    match tokio::time::timeout(limit, read).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            tracing::warn!(
                %error,
                source,
                "cannot read a metrics source; its families are left out of this scrape"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                source,
                timeout_ms = limit.as_millis(),
                "a metrics source did not answer in time; its families are left out of this scrape"
            );
            None
        }
    }
}

/// Reads the procfs file at `path` and parses it with `parse`; `None` when
/// either step fails.
///
/// A plain blocking read on purpose: procfs files are generated from kernel
/// memory and never wait for a disk, so `tokio::fs`, which would hand the
/// read to the blocking thread pool, would only add overhead. The file is
/// read as bytes, because the kernel writes the thread name into it as it
/// is, which need not be UTF-8 ([`proc_stat`]). A missing file is logged at
/// debug level, because it is a fixed property of a system without procfs
/// (a developer's macOS), not an incident. Any other read error and a parse
/// error are warnings, because Linux promises these files and formats.
fn read_proc<T>(path: &str, parse: fn(&[u8]) -> Result<T, ProcParseError>) -> Option<T> {
    let bytes = std::fs::read(path)
        .inspect_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                tracing::debug!(%error, path, "no procfs file; its family is left out");
            } else {
                tracing::warn!(%error, path, "cannot read a procfs file; its family is left out");
            }
        })
        .ok()?;
    parse(&bytes)
        .inspect_err(|error| tracing::warn!(%error, "cannot parse a procfs file"))
        .ok()
}

/// Writes the latest sample on each encoding of the registry.
#[derive(Debug)]
struct RuntimeCollector {
    sample: SharedSample,
}

impl Collector for RuntimeCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), fmt::Error> {
        let sample = latest(&self.sample);
        if let Some(pool) = sample.pool {
            let (name, help) = DB_CONNECTIONS;
            let mut family = encoder.encode_descriptor(name, help, None, MetricType::Gauge)?;
            family
                .encode_family(&[("state", "used")])?
                .encode_gauge(&pool.used)?;
            family
                .encode_family(&[("state", "idle")])?
                .encode_gauge(&pool.idle)?;
        }
        if let Some(queue) = sample.queue {
            let (name, help) = QUEUE_JOBS;
            let mut family = encoder.encode_descriptor(name, help, None, MetricType::Gauge)?;
            for (state, jobs) in JOB_STATES.into_iter().zip(queue.jobs) {
                family
                    .encode_family(&[("state", state)])?
                    .encode_gauge(&jobs)?;
            }
            // Only when such rows exist, so the usual exposition stays at
            // the five states Loco writes.
            if queue.other_jobs > 0 {
                family
                    .encode_family(&[("state", OTHER_JOB_STATE)])?
                    .encode_gauge(&queue.other_jobs)?;
            }
            let (name, help) = QUEUE_OLDEST_AGE;
            let age = ConstGauge::new(queue.oldest_queued_age_seconds);
            age.encode(encoder.encode_descriptor(
                name,
                help,
                Some(&Unit::Seconds),
                age.metric_type(),
            )?)?;
        }
        if let Some(seconds) = sample.cpu_seconds {
            let (name, help) = PROCESS_CPU;
            let cpu = ConstCounter::new(seconds);
            cpu.encode(encoder.encode_descriptor(
                name,
                help,
                Some(&Unit::Seconds),
                cpu.metric_type(),
            )?)?;
        }
        if let Some(bytes) = sample.resident_bytes {
            let (name, help) = PROCESS_RESIDENT_MEMORY;
            let memory = ConstGauge::new(bytes);
            memory.encode(encoder.encode_descriptor(
                name,
                help,
                Some(&Unit::Bytes),
                memory.metric_type(),
            )?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use loco_rs::bgworker::JobStatus;
    use prometheus_client::encoding::text::encode;

    use super::*;
    use crate::domain::runtime_metrics::QUEUED_JOB_STATE;

    /// The exposition of a registry that holds only the runtime families,
    /// with `sample` as their latest reading.
    fn exposition(sample: RuntimeSample) -> String {
        let mut registry = Registry::default();
        let runtime = RuntimeMetrics::register(&mut registry, DatabaseConnection::default());
        *runtime.sample.lock().expect("unpoisoned") = sample;
        let mut body = String::new();
        encode(&mut body, &registry).expect("encode");
        body
    }

    #[test]
    fn a_full_sample_writes_every_family() {
        let body = exposition(RuntimeSample {
            pool: Some(PoolUsage { used: 3, idle: 2 }),
            queue: Some(QueueStats {
                jobs: [4, 1, 30, 2, 0],
                other_jobs: 0,
                oldest_queued_age_seconds: 12.5,
            }),
            cpu_seconds: Some(3190.61),
            resident_bytes: Some(3_227_648),
        });
        assert_eq!(
            body,
            "# HELP db_client_connections Connections of the application's database pool, by \
             state: used (checked out) or idle.\n\
             # TYPE db_client_connections gauge\n\
             db_client_connections{state=\"used\"} 3\n\
             db_client_connections{state=\"idle\"} 2\n\
             # HELP loco_queue_jobs Jobs in the Postgres job queue table pg_loco_queue, by state.\n\
             # TYPE loco_queue_jobs gauge\n\
             loco_queue_jobs{state=\"queued\"} 4\n\
             loco_queue_jobs{state=\"processing\"} 1\n\
             loco_queue_jobs{state=\"completed\"} 30\n\
             loco_queue_jobs{state=\"failed\"} 2\n\
             loco_queue_jobs{state=\"cancelled\"} 0\n\
             # HELP loco_queue_oldest_queued_job_age_seconds Time since the oldest queued job \
             became due, 0 when none is waiting; it grows while no worker takes jobs.\n\
             # TYPE loco_queue_oldest_queued_job_age_seconds gauge\n\
             # UNIT loco_queue_oldest_queued_job_age_seconds seconds\n\
             loco_queue_oldest_queued_job_age_seconds 12.5\n\
             # HELP process_cpu_seconds Total user and system CPU time spent in seconds.\n\
             # TYPE process_cpu_seconds counter\n\
             # UNIT process_cpu_seconds seconds\n\
             process_cpu_seconds_total 3190.61\n\
             # HELP process_resident_memory_bytes Resident memory size in bytes.\n\
             # TYPE process_resident_memory_bytes gauge\n\
             # UNIT process_resident_memory_bytes bytes\n\
             process_resident_memory_bytes 3227648\n\
             # EOF\n"
        );
    }

    #[test]
    fn unknown_job_states_appear_only_when_there_are_some() {
        let queue = |other_jobs| RuntimeSample {
            queue: Some(QueueStats {
                other_jobs,
                ..QueueStats::default()
            }),
            ..RuntimeSample::default()
        };
        assert!(exposition(queue(2)).contains("loco_queue_jobs{state=\"other\"} 2\n"));
        assert!(!exposition(queue(0)).contains("other"));
    }

    #[test]
    fn sources_that_were_not_read_leave_their_families_out() {
        assert_eq!(exposition(RuntimeSample::default()), "# EOF\n");
    }

    #[test]
    fn the_job_states_are_the_ones_loco_writes() {
        let loco = [
            JobStatus::Queued,
            JobStatus::Processing,
            JobStatus::Completed,
            JobStatus::Failed,
            JobStatus::Cancelled,
        ]
        .map(|status| status.to_string());
        assert_eq!(loco, JOB_STATES);
        assert_eq!(JobStatus::Queued.to_string(), QUEUED_JOB_STATE);
    }

    #[tokio::test]
    async fn a_disconnected_database_gives_no_pool_or_queue_sample() {
        let mut registry = Registry::default();
        let runtime = RuntimeMetrics::register(&mut registry, DatabaseConnection::default());
        runtime.refresh().await;
        let sample = latest(&runtime.sample);
        assert_eq!(sample.pool, None);
        assert_eq!(sample.queue, None);
        // The tests run on Linux, where the process families always read.
        assert!(sample.cpu_seconds.is_some_and(|seconds| seconds >= 0.0));
        assert!(sample.resident_bytes.is_some_and(|bytes| bytes > 0));
    }

    #[tokio::test]
    async fn a_source_that_does_not_answer_in_time_gives_no_sample() {
        let read = pending::<Result<u8, fmt::Error>>();
        assert_eq!(bounded(Duration::from_millis(10), read, "test").await, None);
        let ready = async { Ok::<_, fmt::Error>(7) };
        assert_eq!(
            bounded(Duration::from_millis(10), ready, "test").await,
            Some(7)
        );
    }

    #[test]
    fn an_unreadable_proc_file_gives_no_sample() {
        assert_eq!(
            read_proc("/proc/self/no-such-file", proc_stat::resident_bytes),
            None
        );
        // procfs files that exist but hold something else.
        assert_eq!(read_proc(STAT_PATH, proc_stat::resident_bytes), None);
    }
}
