//! A startup packet's settings, read as 18.6 reads them.
//!
//! 18.6 takes settings from two places in the packet, in a fixed order
//! (`utils/init/postinit.c`'s `process_startup_options`): first the `options` string — split
//! by `pg_split_opts` and read as a command line by `tcop/postgres.c`'s
//! `process_postgres_switches` — then every other key, in packet order (`ProcessStartupPacket`
//! keeps each key that is not exactly `database`, `user`, `options` or `replication`, and not
//! a `_pq_.` protocol option). Each assignment goes through `SetConfigOption` as it is read, so
//! 18.6's own check of a value fails the startup at that assignment, and the last assignment of
//! a setting is the session's value.
//!
//! [`startup_steps`] produces that ordered list, and [`apply_startup_steps`] walks it: 18.6's
//! own check of each assignment to a setting pglet models runs where it is read, in order, and
//! pglet's refusals — which stand in for no Postgres error — run afterwards, on the value each
//! setting ends with.
//!
//! @module

use pglet_sql::catalog::{
    SessionSettings, check_guc_value, check_guc_value_postgres, check_search_path_value,
    client_encoding_key, guc_value_is_checked,
};

/// A startup refused: a `FATAL` with this SQLSTATE and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRefusal {
    pub code: &'static str,
    pub message: String,
}

impl StartupRefusal {
    const fn syntax(message: String) -> Self {
        Self {
            code: "42601",
            message,
        }
    }

    fn invalid_argument(word: &str) -> Self {
        Self::syntax(format!(
            "invalid command-line argument for server process: {word}"
        ))
    }
}

impl From<pglet_sql::Error> for StartupRefusal {
    fn from(e: pglet_sql::Error) -> Self {
        Self {
            code: e.sqlstate(),
            message: e.to_string(),
        }
    }
}

/// One step of a startup, in 18.6's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupStep {
    /// `SetConfigOption(name, value)`, the name folded as `find_option` folds it.
    Assign(String, String),
    /// Where 18.6 stops the startup while reading `options`.
    Refuse(StartupRefusal),
}

/// A startup packet's steps: its `options` string's, then its other keys'.
///
/// `options` is matched by its exact spelling, as `ProcessStartupPacket`'s `strcmp` matches it,
/// and a later one replaces an earlier; an `OPTIONS` key is an ordinary setting name. The steps
/// stop at the first refusal.
#[must_use]
pub fn startup_steps(params: &[(String, String)]) -> Vec<StartupStep> {
    let mut steps = params
        .iter()
        .rev()
        .find(|(k, _)| k == "options")
        .map_or_else(Vec::new, |(_, v)| options_steps(v));
    if matches!(steps.last(), Some(StartupStep::Refuse(_))) {
        return steps;
    }
    steps.extend(
        params
            .iter()
            .filter(|(k, _)| {
                !matches!(k.as_str(), "database" | "user" | "options" | "replication")
                    && !k.starts_with("_pq_.")
            })
            .map(|(k, v)| StartupStep::Assign(k.to_ascii_lowercase(), v.clone())),
    );
    steps
}

/// The value a setting ends with, if any step assigns it.
#[must_use]
pub fn final_value<'a>(steps: &'a [StartupStep], name: &str) -> Option<&'a str> {
    steps.iter().rev().find_map(|step| match step {
        StartupStep::Assign(n, v) if n == name => Some(v.as_str()),
        _ => None,
    })
}

/// Walk the steps into `settings`: 18.6's own check of each assignment pglet models, in order,
/// then pglet's refusals of the values the settings end with.
///
/// `lock_timeout` and `default_transaction_isolation` are applied (the last one is the
/// session's value), a setting whose value pglet decides is checked as 18.6 checks it and its
/// final value then decided, `search_path`'s final value gets its `set_config`'s verdict (its
/// one-schema rule), and `application_name` is the caller's, read with [`final_value`]. Every
/// other setting is ignored, as it is through a `SET` (`EDGES.md` rows 67 and 224).
///
/// # Errors
///
/// The first refusal in 18.6's order: a step's own, 18.6's check of an assignment, then
/// pglet's refusal of a final value — `client_encoding` worded as 18.6 words an encoding it
/// will not serve at startup (`0A000` for `mule_internal`, which 18.6 refuses there too;
/// `22023` for the rest), `search_path` its `SET`'s `0A000`.
pub fn apply_startup_steps(
    settings: &mut SessionSettings,
    steps: &[StartupStep],
) -> Result<(), StartupRefusal> {
    let mut decided: Vec<(&str, &str)> = Vec::new();
    for step in steps {
        let (name, value) = match step {
            StartupStep::Refuse(refusal) => return Err(refusal.clone()),
            StartupStep::Assign(name, value) => (name.as_str(), value.as_str()),
        };
        match name {
            "lock_timeout" => settings.apply_startup_lock_timeout(value)?,
            "default_transaction_isolation" => settings.apply_startup_default_isolation(value)?,
            _ if guc_value_is_checked(name) || name == "search_path" => {
                if name != "search_path" {
                    check_guc_value_postgres(name, Some(value))?;
                }
                match decided.iter_mut().find(|(n, _)| *n == name) {
                    Some(entry) => entry.1 = value,
                    None => decided.push((name, value)),
                }
            }
            _ => {}
        }
    }
    for (name, value) in decided {
        if name == "search_path" {
            check_search_path_value(value)?;
            continue;
        }
        if let Err(e) = check_guc_value(name, Some(value)) {
            if name == "client_encoding" && client_encoding_key(value) == "muleinternal" {
                // 18.6 knows the name and refuses it once the session's encoding is set up:
                // `FATAL 0A000`, judged on the value the startup ends with (measured: an
                // earlier `mule_internal` overridden by `UTF8` is served there).
                return Err(StartupRefusal {
                    code: "0A000",
                    message: "conversion between MULE_INTERNAL and UTF8 is not supported"
                        .to_string(),
                });
            }
            if name == "client_encoding" {
                return Err(StartupRefusal {
                    code: "22023",
                    message: format!(
                        "invalid value for parameter \"client_encoding\": \"{value}\""
                    ),
                });
            }
            return Err(e.into());
        }
    }
    Ok(())
}

/// `pg_split_opts`: the words of an `options` string, split at unescaped C `isspace` (space,
/// tab, line feed, vertical tab, form feed, carriage return), a backslash taking the next
/// character literally and a trailing one dropped.
#[must_use]
pub fn split_options(options: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = options.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| is_c_space(*c)) {
            chars.next();
        }
        if chars.peek().is_none() {
            return words;
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if is_c_space(c) {
                break;
            }
            chars.next();
            if c == '\\' {
                word.extend(chars.next());
            } else {
                word.push(c);
            }
        }
        words.push(word);
    }
}

/// C `isspace` in the C locale.
const fn is_c_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// `process_postgres_switches`' option string.
const SWITCHES: &str = "B:bC:c:D:d:EeFf:h:ijk:lN:nOPp:r:S:sTt:v:W:-:";

/// Whether `c` is a switch, and whether it takes an argument.
fn switch_takes_argument(c: char) -> Option<bool> {
    if c == ':' {
        return None;
    }
    let at = SWITCHES.find(c)?;
    Some(SWITCHES[at + c.len_utf8()..].starts_with(':'))
}

/// An `options` string's steps, as `process_postgres_switches` reads its words under glibc's
/// `getopt`: switches cluster (`-Ec`), an argument-taking switch takes the rest of its word or
/// else the next word whatever it looks like, `--` ends the switches, and a word that is not a
/// switch is passed over and refused once every switch is read (glibc permutes it to the end).
///
/// A refused word is named as the word holding the switch; 18.6 names the previous word when
/// an unknown switch is not the last of its cluster — the SQLSTATE is the same.
#[must_use]
pub fn options_steps(options: &str) -> Vec<StartupStep> {
    let words = split_options(options);
    let mut steps = Vec::new();
    let mut not_switches: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = words[i].as_str();
        i += 1;
        if word == "--" {
            not_switches.extend(words[i..].iter().map(String::as_str));
            break;
        }
        let Some(cluster) = word.strip_prefix('-').filter(|rest| !rest.is_empty()) else {
            not_switches.push(word);
            continue;
        };
        let mut rest = cluster;
        while let Some(c) = rest.chars().next() {
            rest = &rest[c.len_utf8()..];
            let Some(takes_argument) = switch_takes_argument(c) else {
                steps.push(StartupStep::Refuse(StartupRefusal::invalid_argument(word)));
                return steps;
            };
            let argument = if !takes_argument {
                None
            } else if !rest.is_empty() {
                Some((std::mem::take(&mut rest), word))
            } else if let Some(next) = words.get(i) {
                i += 1;
                Some((next.as_str(), next.as_str()))
            } else {
                steps.push(StartupStep::Refuse(StartupRefusal::invalid_argument(word)));
                return steps;
            };
            if let Some(step) = switch_step(c, argument) {
                let refused = matches!(step, StartupStep::Refuse(_));
                steps.push(step);
                if refused {
                    return steps;
                }
            }
        }
    }
    if let Some(word) = not_switches.first() {
        steps.push(StartupStep::Refuse(StartupRefusal::invalid_argument(word)));
    }
    steps
}

/// What one switch does from a client's packet — `process_postgres_switches`' non-`secure`
/// arms. `argument` is the argument and the word it was read from.
fn switch_step(switch: char, argument: Option<(&str, &str)>) -> Option<StartupStep> {
    let assign =
        |name: &str, value: &str| Some(StartupStep::Assign(name.to_string(), value.to_string()));
    let (arg, arg_word) = argument.unwrap_or(("", ""));
    match switch {
        'c' | '-' => Some(long_option(switch, arg)),
        'B' => assign("shared_buffers", arg),
        'e' => assign("datestyle", "euro"),
        'F' => assign("fsync", "false"),
        'h' => assign("listen_addresses", arg),
        'i' => assign("listen_addresses", "*"),
        'k' => assign("unix_socket_directories", arg),
        'l' => assign("ssl", "true"),
        'N' => assign("max_connections", arg),
        'O' => assign("allow_system_table_mods", "true"),
        'P' => assign("ignore_system_indexes", "true"),
        'p' => assign("port", arg),
        'S' => assign("work_mem", arg),
        's' => assign("log_statement_stats", "true"),
        'W' => assign("post_auth_delay", arg),
        // `set_plan_disabling_options`: the argument's first letter names a plan type.
        'f' => {
            let setting = match arg.chars().next() {
                Some('s') => "enable_seqscan",
                Some('i') => "enable_indexscan",
                Some('o') => "enable_indexonlyscan",
                Some('b') => "enable_bitmapscan",
                Some('t') => "enable_tidscan",
                Some('n') => "enable_nestloop",
                Some('m') => "enable_mergejoin",
                Some('h') => "enable_hashjoin",
                _ => {
                    return Some(StartupStep::Refuse(StartupRefusal::invalid_argument(
                        arg_word,
                    )));
                }
            };
            assign(setting, "false")
        }
        // `get_stats_option_name`.
        't' => {
            let setting = if arg.starts_with("pa") {
                "log_parser_stats"
            } else if arg.starts_with("pl") {
                "log_planner_stats"
            } else if arg.starts_with('e') {
                "log_executor_stats"
            } else {
                return Some(StartupStep::Refuse(StartupRefusal::invalid_argument(
                    arg_word,
                )));
            };
            assign(setting, "true")
        }
        // `-d` sets logging levels through `set_debug_options` (18.6 refuses `-d 6` and above,
        // `22023`, a setting pglet does not model — `EDGES.md` row 67), and `-b`, `-C`, `-D`, `-E`, `-j`, `-n`, `-r`, `-T` and `-v` do nothing outside a
        // postmaster or single-user command line: none reaches a setting pglet reads.
        _ => None,
    }
}

/// `-c name=value` / `--name=value`: `ParseLongOption`, whose name reads each `-` as `_`.
fn long_option(switch: char, arg: &str) -> StartupStep {
    // `parse_dispatch_option`: a subprogram's switch is refused out of first place.
    if switch == '-' && matches!(arg, "check" | "boot" | "describe-config" | "single") {
        return StartupStep::Refuse(StartupRefusal::syntax(format!(
            "--{arg} must be first argument"
        )));
    }
    let Some((name, value)) = arg.split_once('=') else {
        let spelled = if switch == '-' {
            format!("--{arg}")
        } else {
            format!("-c {arg}")
        };
        return StartupStep::Refuse(StartupRefusal::syntax(format!(
            "{spelled} requires a value"
        )));
    };
    StartupStep::Assign(
        name.replace('-', "_").to_ascii_lowercase(),
        value.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::{StartupStep, options_steps, split_options};

    fn assigns(options: &str) -> Vec<(String, String)> {
        options_steps(options)
            .into_iter()
            .map(|step| match step {
                StartupStep::Assign(n, v) => (n, v),
                StartupStep::Refuse(r) => (format!("REFUSE {}", r.code), r.message),
            })
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    /// `pg_split_opts`: C `isspace` (the vertical tab included), backslash escapes, a
    /// trailing backslash dropped.
    #[test]
    fn options_split_as_pg_split_opts_splits_them() {
        assert_eq!(
            split_options(" a\x0bb\\ c\td\\\\e\x0cf\rg\n h\\"),
            ["a", "b c", "d\\e", "f", "g", "h"]
        );
        assert!(split_options(" \t ").is_empty());
    }

    /// Each answer is what 18.6 does with the same `options` in a raw v3 startup packet
    /// (measured): a clustered switch, a switch's argument taken from the next word, `--`,
    /// the plan and stats switches, and the refusals with their SQLSTATEs.
    #[test]
    fn options_read_as_getopt_reads_them() {
        let q = "quote_all_identifiers";
        assert_eq!(assigns("-Ec quote_all_identifiers=on"), pairs(&[(q, "on")]));
        assert_eq!(
            assigns("-ec quote_all_identifiers=on"),
            pairs(&[("datestyle", "euro"), (q, "on")])
        );
        assert_eq!(
            assigns("-c\x0bquote_all_identifiers=on"),
            pairs(&[(q, "on")])
        );
        assert_eq!(assigns("-d -cquote_all_identifiers=on"), pairs(&[]));
        assert_eq!(
            assigns("-S -cquote_all_identifiers=on"),
            pairs(&[("work_mem", "-cquote_all_identifiers=on")])
        );
        assert_eq!(
            assigns("--application-name=hello"),
            pairs(&[("application_name", "hello")])
        );
        assert_eq!(
            assigns("-cApplication-Name=Z"),
            pairs(&[("application_name", "Z")])
        );
        assert_eq!(
            assigns("-c application_name=x --"),
            pairs(&[("application_name", "x")])
        );
        assert_eq!(
            assigns("-Eb -f s -t pa -P -D /tmp -v 3 -r f -T -n -j -C x -c application_name=x"),
            pairs(&[
                ("enable_seqscan", "false"),
                ("log_parser_stats", "true"),
                ("ignore_system_indexes", "true"),
                ("application_name", "x"),
            ])
        );
        let refused = |code: &str, message: &str| ((format!("REFUSE {code}")), message.to_string());
        let arg = "invalid command-line argument for server process: ";
        assert_eq!(
            assigns("foo -c application_name=x"),
            vec![
                ("application_name".to_string(), "x".to_string()),
                refused("42601", &format!("{arg}foo"))
            ]
        );
        assert_eq!(
            assigns("-Z -c application_name=x"),
            vec![refused("42601", &format!("{arg}-Z"))]
        );
        assert_eq!(assigns("-c"), vec![refused("42601", &format!("{arg}-c"))]);
        assert_eq!(assigns("-"), vec![refused("42601", &format!("{arg}-"))]);
        assert_eq!(
            assigns("-- -c application_name=x"),
            vec![refused("42601", &format!("{arg}-c"))]
        );
        assert_eq!(
            assigns("-c quote_all_identifiers"),
            vec![refused(
                "42601",
                "-c quote_all_identifiers requires a value"
            )]
        );
        assert_eq!(
            assigns("--quote_all_identifiers"),
            vec![refused("42601", "--quote_all_identifiers requires a value")]
        );
        assert_eq!(
            assigns("--single"),
            vec![refused("42601", "--single must be first argument")]
        );
        assert_eq!(assigns("-f z"), vec![refused("42601", &format!("{arg}z"))]);
    }
}
