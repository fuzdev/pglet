//! The server's command line, parsed in one pass and refused loud.
//!
//! Every argument is a flag this binary knows, a value one of them takes, or the `dump`
//! subcommand in first position — anything else is an error naming it. A flag's value is
//! validated where it is read, so a mistyped flag or value never falls back to a default:
//! `--data_dir x` starting a server that keeps nothing, or `--port abc` binding the default
//! port, is the data-loss shape this exists to refuse.

use std::path::PathBuf;

/// The storage backend `--storage` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    /// `MemoryStorage`: no MVCC and no persistence.
    Memory,
    /// The CoW B+tree, in memory unless `--data-dir` makes it persistent.
    Btree,
}

/// What the command line asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// `--help` / `-h`.
    pub help: bool,
    /// The `dump` subcommand.
    pub dump: bool,
    /// `--port`; the caller defaults it.
    pub port: Option<u16>,
    /// `--storage`; the caller defaults it.
    pub storage: Option<StorageKind>,
    /// `--data-dir`.
    pub data_dir: Option<PathBuf>,
    /// `--coerce-unknown-types`.
    pub coerce_unknown_types: bool,
    /// `--lock-timeout`, in milliseconds, read as a `lock_timeout` setting value is.
    pub lock_timeout_ms: Option<u64>,
}

/// Parse the arguments after the program name.
///
/// # Errors
///
/// A message naming the argument, for an unknown argument, a flag given twice, a flag missing
/// its value (a following `--…` is not taken as one), an unparsable `--port`, `--storage` or
/// `--lock-timeout`, `--storage memory` with `--data-dir`, a server flag given to `dump`, and a
/// `dump` without `--data-dir`.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.into_iter().peekable();
    if args.peek().map(String::as_str) == Some("dump") {
        args.next();
        out.dump = true;
    }
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| match args.next() {
            Some(v) if !v.starts_with("--") => Ok(v),
            _ => Err(format!("{flag} requires a value")),
        };
        match arg.as_str() {
            "--help" | "-h" => out.help = true,
            "--port" => {
                let v = value("--port")?;
                let port = v
                    .parse::<u16>()
                    .map_err(|_| format!("--port: invalid port {v:?}"))?;
                set_once(&mut out.port, port, "--port")?;
            }
            "--storage" => {
                let v = value("--storage")?;
                let kind = match v.as_str() {
                    "memory" => StorageKind::Memory,
                    "btree" => StorageKind::Btree,
                    _ => {
                        return Err(format!(
                            "--storage: unknown storage type {v:?} (memory or btree)"
                        ));
                    }
                };
                set_once(&mut out.storage, kind, "--storage")?;
            }
            "--data-dir" => {
                let v = value("--data-dir")?;
                set_once(&mut out.data_dir, PathBuf::from(v), "--data-dir")?;
            }
            "--lock-timeout" => {
                let v = value("--lock-timeout")?;
                let ms = pglet_sql::catalog::parse_lock_timeout(&v)
                    .map_err(|e| format!("--lock-timeout: {e}"))?;
                set_once(&mut out.lock_timeout_ms, ms, "--lock-timeout")?;
            }
            "--coerce-unknown-types" => {
                if out.coerce_unknown_types {
                    return Err("--coerce-unknown-types given twice".into());
                }
                out.coerce_unknown_types = true;
            }
            _ => return Err(format!("unknown argument {arg:?} (see --help)")),
        }
    }
    if out.storage == Some(StorageKind::Memory) && out.data_dir.is_some() {
        return Err("--storage memory keeps nothing; it cannot take --data-dir".into());
    }
    if out.dump
        && (out.port.is_some()
            || out.storage.is_some()
            || out.coerce_unknown_types
            || out.lock_timeout_ms.is_some())
    {
        return Err("dump takes only --data-dir".into());
    }
    if out.dump && !out.help && out.data_dir.is_none() {
        return Err("dump requires --data-dir <PATH>".into());
    }
    Ok(out)
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{flag} given twice"));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn empty_is_all_defaults() {
        assert_eq!(parse(&[]), Ok(Args::default()));
    }

    #[test]
    fn every_flag_reads_its_value() {
        let args = parse(&[
            "--port",
            "0",
            "--storage",
            "btree",
            "--data-dir",
            "/tmp/x",
            "--lock-timeout",
            "500ms",
            "--coerce-unknown-types",
        ])
        .unwrap();
        assert_eq!(args.port, Some(0));
        assert_eq!(args.storage, Some(StorageKind::Btree));
        assert_eq!(args.data_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(args.lock_timeout_ms, Some(500));
        assert!(args.coerce_unknown_types);
        assert!(!args.dump && !args.help);
    }

    #[test]
    fn a_mistyped_flag_is_refused_not_ignored() {
        // The data-loss shape: `--data_dir` once started a server that persisted nothing.
        let err = parse(&["--data_dir", "/tmp/x"]).unwrap_err();
        assert!(err.contains("\"--data_dir\""), "{err}");
        assert!(parse(&["--datadir", "/tmp/x"]).is_err());
        assert!(parse(&["stray"]).is_err());
    }

    #[test]
    fn an_unparsable_value_is_refused_not_defaulted() {
        assert!(parse(&["--port", "abc"]).unwrap_err().contains("--port"));
        assert!(parse(&["--port", "70000"]).is_err());
        assert!(parse(&["--storage", "disk"]).unwrap_err().contains("disk"));
    }

    #[test]
    fn an_unparsable_lock_timeout_is_refused_here() {
        assert!(
            parse(&["--lock-timeout", "bogus"])
                .unwrap_err()
                .contains("--lock-timeout")
        );
        assert_eq!(
            parse(&["--lock-timeout", "0"]).unwrap().lock_timeout_ms,
            Some(0)
        );
    }

    #[test]
    fn a_missing_value_is_refused() {
        assert!(parse(&["--port"]).unwrap_err().contains("requires a value"));
        // A following flag is not a value: `--data-dir --port 5434` is not a directory named
        // `--port`.
        assert!(
            parse(&["--data-dir", "--port", "5434"])
                .unwrap_err()
                .contains("--data-dir requires a value")
        );
    }

    #[test]
    fn a_repeated_flag_is_refused() {
        assert!(
            parse(&["--port", "1", "--port", "2"])
                .unwrap_err()
                .contains("twice")
        );
        assert!(parse(&["--coerce-unknown-types", "--coerce-unknown-types"]).is_err());
    }

    #[test]
    fn memory_storage_with_a_data_dir_is_refused() {
        assert!(parse(&["--storage", "memory", "--data-dir", "/tmp/x"]).is_err());
        assert_eq!(
            parse(&["--storage", "memory"]).unwrap().storage,
            Some(StorageKind::Memory)
        );
    }

    #[test]
    fn help_is_a_flag_not_a_value() {
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-h", "--port", "1"]).unwrap().help);
        // `--help` where a value belongs is a missing value, not a request for help.
        assert!(parse(&["--port", "--help"]).is_err());
    }

    #[test]
    fn dump_is_the_first_argument_and_takes_only_a_data_dir() {
        let args = parse(&["dump", "--data-dir", "/tmp/x"]).unwrap();
        assert!(args.dump);
        assert_eq!(args.data_dir, Some(PathBuf::from("/tmp/x")));
        assert!(parse(&["dump", "--port", "1"]).is_err());
        assert!(parse(&["--port", "1", "dump"]).is_err());
        assert!(
            parse(&["dump"])
                .unwrap_err()
                .contains("requires --data-dir")
        );
        assert!(parse(&["dump", "--help"]).unwrap().help);
    }
}
