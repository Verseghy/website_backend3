//! Parsers for the two `/proc/self` files behind backend3's process metrics:
//! CPU time and resident memory (ADR 0023 §9).
//!
//! The caller (`controllers::metrics::runtime`) reads the files and hands
//! their bytes here, so the parsing is tested over captured samples on any
//! machine, without a Linux kernel (ADR 0008 rule 1). The parsers read only
//! the fields they need and fail with a [`ProcParseError`] that names the
//! file and the field, never with a panic: a scrape that cannot parse a file
//! leaves that family out.
//!
//! - `/proc/self/stat` (`proc_pid_stat(5)`) is one line,
//!   `pid (comm) state ppid ...`. `comm` is the thread name, which any code
//!   in the process can set, so it may hold spaces and `)` itself; the fields
//!   are therefore counted from the *last* `)`. Fields 14 and 15, `utime` and
//!   `stime`, are the CPU time spent in user and in kernel mode, in clock
//!   ticks of [`USER_HZ`].
//! - `/proc/self/status` (`proc_pid_status(5)`) has one `Key:\tvalue` line
//!   per entry. `VmRSS` is the resident set size in `kB`, which the kernel
//!   means as KiB. Memory comes from here rather than from `stat`'s `rss`
//!   field, which counts pages and would need the page size.
//! - Neither file is necessarily UTF-8. Both carry the thread name, whose
//!   bytes the kernel does not check, and which it cuts at 15 bytes, even
//!   inside a character. The parsers decode lossily: a replacement
//!   character changes only the name, which they skip, and never a `)`,
//!   which no multi-byte UTF-8 sequence contains.

/// The clock ticks per second of `utime` and `stime` (`USER_HZ`, what
/// `sysconf(_SC_CLK_TCK)` returns).
///
/// Linux fixes `USER_HZ` at 100 on x86-64, the only target of backend3's
/// image (Containerfile), whatever the kernel's internal tick rate. Reading
/// it at run time would need libc's `sysconf`, an `unsafe` call, and the
/// workspace forbids `unsafe` code; the Prometheus Go client's `procfs`
/// package hard-codes 100 for the same reason.
pub const USER_HZ: u32 = 100;

/// The 0-based index of `utime` among the whitespace-separated fields that
/// follow the command name in `/proc/self/stat`: those start at field 3
/// (`state`), and `utime` is field 14. `stime` follows it.
const UTIME_INDEX_AFTER_COMM: usize = 11;

/// The key of the resident set size in `/proc/self/status`.
const VM_RSS_KEY: &str = "VmRSS:";

/// Why a `/proc/self` file could not be parsed. The `Display` text names the
/// file and the field, for the log line of the scrape that skipped it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcParseError {
    /// `/proc/self/stat` has no `)` that ends the command name.
    #[error("/proc/self/stat has no `)` after the command name")]
    NoCommandEnd,
    /// A field the parser needs is missing.
    #[error("{file} has no {field} field")]
    MissingField {
        /// The file, for example `/proc/self/stat`.
        file: &'static str,
        /// The field, for example `utime`.
        field: &'static str,
    },
    /// A field is not a plain decimal number.
    #[error("{file} field {field} is not a decimal number: {value:?}")]
    NotANumber {
        /// The file, for example `/proc/self/stat`.
        file: &'static str,
        /// The field, for example `utime`.
        field: &'static str,
        /// The text found in its place.
        value: String,
    },
    /// A value does not fit in 64 bits once it is converted.
    #[error("{file} field {field} overflows 64 bits")]
    Overflow {
        /// The file, for example `/proc/self/status`.
        file: &'static str,
        /// The field, for example `VmRSS`.
        field: &'static str,
    },
    /// `VmRSS` carries a unit other than `kB`.
    #[error("/proc/self/status field VmRSS has the unit {unit:?}, not kB")]
    UnexpectedUnit {
        /// The unit found, empty when there was none.
        unit: String,
    },
}

/// Parses one decimal field, `token`, of `file`.
///
/// Only ASCII digits are accepted: `str::parse` would also take a leading
/// `+`, which the kernel never writes, so such a file is not what it seems.
fn decimal(
    token: Option<&str>,
    file: &'static str,
    field: &'static str,
) -> Result<u64, ProcParseError> {
    let token = token.ok_or(ProcParseError::MissingField { file, field })?;
    let not_a_number = || ProcParseError::NotANumber {
        file,
        field,
        value: token.to_owned(),
    };
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(not_a_number());
    }
    // Only digits are left, so the one remaining failure is overflow.
    token
        .parse()
        .map_err(|_| ProcParseError::Overflow { file, field })
}

/// Returns the CPU time of the process, user plus kernel mode, in seconds.
///
/// - `stat`: the bytes of `/proc/self/stat`.
///
/// # Errors
///
/// Returns a [`ProcParseError`] when the command name is not closed, when
/// `utime` or `stime` is missing or not a decimal number, or when their sum
/// overflows.
pub fn cpu_seconds(stat: &[u8]) -> Result<f64, ProcParseError> {
    const FILE: &str = "/proc/self/stat";
    let stat = String::from_utf8_lossy(stat);
    let (_, after_comm) = stat.rsplit_once(')').ok_or(ProcParseError::NoCommandEnd)?;
    let mut fields = after_comm.split_ascii_whitespace();
    let utime = decimal(fields.nth(UTIME_INDEX_AFTER_COMM), FILE, "utime")?;
    let stime = decimal(fields.next(), FILE, "stime")?;
    let ticks = utime.checked_add(stime).ok_or(ProcParseError::Overflow {
        file: FILE,
        field: "utime + stime",
    })?;
    #[allow(
        clippy::cast_precision_loss,
        reason = "f64 is exact up to 2^53 ticks, about 2.8 million years of CPU time at 100 Hz"
    )]
    let seconds = ticks as f64 / f64::from(USER_HZ);
    Ok(seconds)
}

/// Returns the resident set size of the process, in bytes.
///
/// - `status`: the bytes of `/proc/self/status`.
///
/// # Errors
///
/// Returns a [`ProcParseError`] when there is no `VmRSS` line (the kernel
/// writes none for kernel threads), when its value is not a decimal number,
/// when its unit is not `kB`, or when the byte count overflows.
pub fn resident_bytes(status: &[u8]) -> Result<u64, ProcParseError> {
    const FILE: &str = "/proc/self/status";
    let status = String::from_utf8_lossy(status);
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix(VM_RSS_KEY))
        .ok_or(ProcParseError::MissingField {
            file: FILE,
            field: "VmRSS",
        })?;
    let mut parts = value.split_ascii_whitespace();
    let kib = decimal(parts.next(), FILE, "VmRSS")?;
    let unit: Vec<&str> = parts.collect();
    if unit != ["kB"] {
        return Err(ProcParseError::UnexpectedUnit {
            unit: unit.join(" "),
        });
    }
    kib.checked_mul(1024).ok_or(ProcParseError::Overflow {
        file: FILE,
        field: "VmRSS",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// `/proc/<pid>/stat` of a long-running process whose name holds a
    /// space, captured on Linux 7.2: utime 313640 and stime 5421 ticks.
    const STAT_WITH_SPACE: &str = "3825 (RDD Process) S 3784 3678 3678 0 -1 4194624 2804043 0 \
        20 0 313640 5421 0 0 20 0 11 0 4581 849727488 39756 18446744073709551615 \
        94118802083840 94118802429389 140730872298400 0 0 0 0 69634 1073745656 0 0 0 17 6 0 0 \
        0 0 0 94118802581000 94118802590388 94118957486080 140730872300079 140730872300347 \
        140730872300347 140730872303581 0\n";

    /// `/proc/<pid>/stat` of a process whose name is itself in parentheses,
    /// captured on Linux 7.2: no CPU time yet.
    const STAT_WITH_PARENS: &str = "2442 ((sd-pam)) S 2440 2440 2440 0 -1 4194624 45 0 0 0 0 0 \
        0 0 20 0 1 0 3599 16900096 741 18446744073709551615 1 1 0 0 0 0 0 4096 0 0 0 0 17 14 0 \
        0 0 0 0 0 0 0 0 0 0 0 0\n";

    /// The start of `/proc/self/status`, captured on Linux 7.2 (the lines
    /// after `VmLib` are left out).
    const STATUS: &str = "Name:\tsed\nUmask:\t0022\nState:\tR (running)\nTgid:\t1673937\n\
        Ngid:\t0\nPid:\t1673937\nPPid:\t1673932\nTracerPid:\t0\nUid:\t1000\t1000\t1000\t1000\n\
        Gid:\t1000\t1000\t1000\t1000\nFDSize:\t64\nGroups:\t10 135 983 1000 \nNStgid:\t1673937\n\
        NSpid:\t1673937\nNSpgid:\t1673932\nNSsid:\t1673932\nKthread:\t0\nVmPeak:\t  231788 kB\n\
        VmSize:\t  231788 kB\nVmLck:\t       0 kB\nVmPin:\t       0 kB\nVmHWM:\t    3152 kB\n\
        VmRSS:\t    3152 kB\nRssAnon:\t     160 kB\nRssFile:\t    2992 kB\n\
        RssShmem:\t       0 kB\nVmData:\t     240 kB\nVmStk:\t     136 kB\nVmExe:\t      84 kB\n\
        VmLib:\t    2480 kB\n";

    #[test]
    fn cpu_time_is_user_plus_kernel_ticks_over_user_hz() {
        assert!((cpu_seconds(STAT_WITH_SPACE.as_bytes()).expect("parse") - 3190.61).abs() < 1e-9);
        assert!(
            cpu_seconds(STAT_WITH_PARENS.as_bytes())
                .expect("parse")
                .abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn fields_are_counted_from_the_last_parenthesis() {
        let stat = "7 (a) 1 2 3 4 5 6 7 8 9 10 (b)) R 1 1 1 0 -1 0 0 0 0 0 250 50 0 0\n";
        assert!((cpu_seconds(stat.as_bytes()).expect("parse") - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_name_that_is_not_utf8_moves_no_field() {
        // A name cut inside a two-byte character, and bytes that are no
        // UTF-8 at all, as `prctl(PR_SET_NAME)` may set them.
        let stat = b"7 (caf\xc3) S 1 1 1 0 -1 0 0 0 0 0 250 50 0 0\n";
        assert!((cpu_seconds(stat).expect("parse") - 3.0).abs() < 1e-9);
        let stat = b"7 (\xff\xfe)\x80) S 1 1 1 0 -1 0 0 0 0 0 1 2 0 0\n";
        assert!((cpu_seconds(stat).expect("parse") - 0.03).abs() < 1e-9);
        assert_eq!(resident_bytes(b"Name:\t\xff\nVmRSS:\t4 kB\n"), Ok(4096));
    }

    #[test]
    fn a_broken_stat_names_what_is_wrong() {
        assert_eq!(cpu_seconds(b""), Err(ProcParseError::NoCommandEnd));
        assert_eq!(
            cpu_seconds(b"1 (x) S 0 0 0 0 0 0 0 0 0 0 7"),
            Err(ProcParseError::MissingField {
                file: "/proc/self/stat",
                field: "stime"
            })
        );
        assert_eq!(
            cpu_seconds(b"1 (x) S 0 0 0 0 0 0 0 0 0 0 +7 1"),
            Err(ProcParseError::NotANumber {
                file: "/proc/self/stat",
                field: "utime",
                value: "+7".to_owned()
            })
        );
        let max = u64::MAX;
        assert_eq!(
            cpu_seconds(format!("1 (x) S 0 0 0 0 0 0 0 0 0 0 {max} 1").as_bytes()),
            Err(ProcParseError::Overflow {
                file: "/proc/self/stat",
                field: "utime + stime"
            })
        );
        assert_eq!(
            cpu_seconds(format!("1 (x) S 0 0 0 0 0 0 0 0 0 0 {max}0 1").as_bytes())
                .map_err(|e| e.to_string()),
            Err("/proc/self/stat field utime overflows 64 bits".to_owned())
        );
    }

    #[test]
    fn resident_memory_is_vm_rss_in_kib() {
        assert_eq!(resident_bytes(STATUS.as_bytes()), Ok(3152 * 1024));
        assert_eq!(resident_bytes(b"VmRSS:\t0 kB"), Ok(0));
    }

    #[test]
    fn a_broken_status_names_what_is_wrong() {
        let no_rss = STATUS.replace("VmRSS", "VmXYZ");
        assert_eq!(
            resident_bytes(no_rss.as_bytes()).map_err(|e| e.to_string()),
            Err("/proc/self/status has no VmRSS field".to_owned())
        );
        assert_eq!(
            resident_bytes(b"VmRSS:\t12 MB\n"),
            Err(ProcParseError::UnexpectedUnit {
                unit: "MB".to_owned()
            })
        );
        assert_eq!(
            resident_bytes(b"VmRSS:\t12\n"),
            Err(ProcParseError::UnexpectedUnit {
                unit: String::new()
            })
        );
        assert_eq!(
            resident_bytes(b"VmRSS:\t12 kB extra\n"),
            Err(ProcParseError::UnexpectedUnit {
                unit: "kB extra".to_owned()
            })
        );
        assert!(matches!(
            resident_bytes(b"VmRSS:\t-1 kB\n"),
            Err(ProcParseError::NotANumber { .. })
        ));
        assert_eq!(
            resident_bytes(format!("VmRSS:\t{} kB\n", u64::MAX / 1024 + 1).as_bytes()),
            Err(ProcParseError::Overflow {
                file: "/proc/self/status",
                field: "VmRSS"
            })
        );
    }

    proptest! {
        #[test]
        fn the_parsers_never_panic(bytes in any::<Vec<u8>>()) {
            let _ = cpu_seconds(&bytes);
            let _ = resident_bytes(&bytes);
        }

        #[test]
        fn cpu_seconds_reads_utime_and_stime_whatever_the_name(
            pid in any::<u32>(),
            // Any bytes, as the kernel does not check them.
            comm in any::<Vec<u8>>(),
            utime in any::<u32>(),
            stime in any::<u32>(),
            tail in proptest::collection::vec(any::<u64>(), 0..40),
        ) {
            let tail: Vec<String> = tail.iter().map(u64::to_string).collect();
            let mut stat = format!("{pid} (").into_bytes();
            stat.extend_from_slice(&comm);
            stat.extend_from_slice(
                format!(") S 1 1 1 0 -1 4194304 0 0 0 0 {utime} {stime} {}\n", tail.join(" "))
                    .as_bytes(),
            );
            // Exact in f64: both are below 2^32, so their sum is below 2^53.
            let expected = (f64::from(utime) + f64::from(stime)) / f64::from(USER_HZ);
            prop_assert_eq!(cpu_seconds(&stat), Ok(expected));
        }

        #[test]
        fn resident_bytes_reads_vm_rss_among_other_lines(
            kib in 0..=u64::MAX / 1024,
            // Keys that cannot be `VmRSS`, so the line below is the only one.
            before in proptest::collection::vec("Rss[A-Za-z]{0,8}:\t[0-9]{1,9} kB", 0..8),
        ) {
            let status = format!("{}\nVmRSS:\t {kib} kB\nRssAnon:\t1 kB\n", before.join("\n"));
            prop_assert_eq!(resident_bytes(status.as_bytes()), Ok(kib * 1024));
        }
    }
}
