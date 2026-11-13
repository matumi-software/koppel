//! Koppel's settings and commands, as one typed [`Settings`] built once at start by clap.
//!
//! A setting is a command line option (`--ui-port 5250`) or an environment variable (`KOPPEL_UI_PORT`),
//! the option winning. The commands are `serve` (the default), `reset-password`, `healthcheck` and
//! `diagnostics` and `prepare-data-folder`.
//!
//! 0.1.x's spellings keep working, as a deprecated alias that is logged once: `Ports__Ui` in the
//! environment and `--Ports:Ui=5250` or `/Ports:Ui 5250` on the command line, matched without regard
//! to case as .NET's configuration did, and `ASPNETCORE_ENVIRONMENT`. [`Normalized`] rewrites them to
//! the new options before clap reads anything, so there is one parser and one set of rules. The old
//! `--reset-password` and `--healthcheck` flags are the commands of the same names. A `KOPPEL_` or
//! `Section__Key` variable that Koppel does not know (a misspelt `Ports__Uii`) is warned about with
//! the nearest known name, never silently ignored. 0.1.x's `appsettings.json` held only log levels, so
//! there is no file to read.

use std::ffi::OsString;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use clap::{Args, CommandFactory, Parser, Subcommand};
use koppel_config::KoppelPaths;
use koppel_web::UiReach;

/// What `koppel --version` prints after the name: the version, the git commit and the build date, from
/// `build.rs`.
const VERSION: &str = env!("KOPPEL_LONG_VERSION");

/// 0.1.x's name for each setting, by the option's long name. A name with `:` (or `__`, in the
/// environment) is a section and a key.
const LEGACY: &[(&str, &str)] = &[
    ("ui-port", "Ports:Ui"),
    ("network-port", "Ports:Network"),
    ("listen-address", "Ports:Address"),
    ("ui-allow-from", "Ui:AllowFrom"),
    ("ui-setup-window-minutes", "Ui:SetupWindowMinutes"),
    ("config-dir", "Paths:Config"),
    ("keys-dir", "Paths:Keys"),
    ("key-certificate", "Paths:KeyCertificate"),
    ("rate-auth-per-minute", "RateLimits:AuthPerMinute"),
    ("rate-token-per-minute", "RateLimits:TokenPerMinute"),
    ("rate-pair-per-minute", "RateLimits:PairPerMinute"),
    ("discovery-enabled", "Discovery:Enabled"),
    ("discovery-port", "Discovery:Port"),
    ("usb-sys-root", "Usb:SysRoot"),
    ("usb-upstream", "Usb:Upstream"),
    ("usb-usbip-path", "Usb:UsbipPath"),
    ("usb-attach-timeout-seconds", "Usb:AttachTimeoutSeconds"),
    ("gateway-port", "Gateway:Port"),
    ("gateway-address", "Gateway:Address"),
    ("gateway-max-sessions", "Gateway:MaxSessions"),
    ("environment", "ASPNETCORE_ENVIRONMENT"),
    ("environment", "DOTNET_ENVIRONMENT"),
];

/// `KOPPEL_` names that other parts of Koppel or its tools read, which are not settings.
const OTHER_KOPPEL_NAMES: &[&str] = &[
    "KOPPEL_RUNNING_IN_CONTAINER",
    "KOPPEL_UI_URL",
    "KOPPEL_HOSTNAME",
    "KOPPEL_IMAGE",
];
/// `KOPPEL_` prefixes of the same kind.
const OTHER_KOPPEL_PREFIXES: &[&str] = &["KOPPEL_CONTRACT_"];

/// What to run.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq, Default)]
pub enum Command {
    /// Run Koppel. This is what no command means, and what the service and the image start.
    #[default]
    Serve,
    /// Clear the UI password and end every session, then exit.
    ResetPassword,
    /// Ask a running Koppel's /healthz and exit 0 when it answers, 1 when it does not.
    Healthcheck,
    /// Write a report to attach to a support request: version, OS, config files with secrets removed,
    /// USB state and the end of the log. Run it as an administrator or root.
    Diagnostics {
        /// The file to write. Standard output unless given.
        file: Option<String>,
    },
    /// Windows only: make the data folder (%ProgramData%\Koppel unless a path is given) usable by SYSTEM
    /// and administrators only, or with --check only verify it. The installer runs this as SYSTEM.
    /// Exits 0 when the folder is safe, 1 when it is not or the call fails.
    PrepareDataFolder {
        /// Only verify the folder and change nothing.
        #[arg(long)]
        check: bool,
        /// The folder. %ProgramData%\Koppel unless given.
        path: Option<String>,
    },
}

impl Command {
    fn from_flag(flag: &str) -> Option<(&'static str, Self)> {
        match flag.to_ascii_lowercase().as_str() {
            "--reset-password" => Some(("reset-password", Self::ResetPassword)),
            "--healthcheck" => Some(("healthcheck", Self::Healthcheck)),
            _ => None,
        }
    }
}

/// USB devices shared over the network.
#[derive(Parser, Debug)]
#[command(name = "koppel", version = VERSION, args_override_self = true)]
struct Cli {
    #[command(flatten)]
    settings: Settings,
    #[command(subcommand)]
    command: Option<Command>,
}

/// Every setting. Each one is also an environment variable, `KOPPEL_` and its name in capitals with
/// `_` between words, and the option wins over it.
#[derive(Args, Debug, Clone, Default)]
pub struct Settings {
    /// The UI's HTTPS port. 5240 unless set.
    #[arg(long, global = true, env = "KOPPEL_UI_PORT", value_name = "PORT", value_parser = blank_ok(parse_port))]
    ui_port: Option<Maybe<u16>>,
    /// The port for what other Koppel machines do. The UI's port unless set.
    #[arg(long, global = true, env = "KOPPEL_NETWORK_PORT", value_name = "PORT", value_parser = blank_ok(parse_port))]
    network_port: Option<Maybe<u16>>,
    /// The IP address the web ports bind to. Every address unless set.
    #[arg(long, global = true, env = "KOPPEL_LISTEN_ADDRESS", value_name = "ADDRESS", value_parser = blank_ok(parse_address))]
    listen_address: Option<Maybe<IpAddr>>,
    /// Who may use the UI: local, lan or any. lan unless set, local in development.
    #[arg(long, global = true, env = "KOPPEL_UI_ALLOW_FROM", value_name = "WHO", value_parser = blank_ok(parse_reach))]
    ui_allow_from: Option<Maybe<UiReach>>,
    /// Minutes a machine without a UI password has to set its first one, 1 to 1440. 10 unless set.
    #[arg(long, global = true, env = "KOPPEL_UI_SETUP_WINDOW_MINUTES", value_name = "MINUTES", value_parser = blank_ok(parse_minutes))]
    ui_setup_window_minutes: Option<Maybe<u64>>,
    /// The folder for the config files. The platform's default unless set.
    #[arg(long, global = true, env = "KOPPEL_CONFIG_DIR", value_name = "DIR")]
    config_dir: Option<String>,
    /// The folder for what seals the secret files. The platform's default unless set.
    #[arg(long, global = true, env = "KOPPEL_KEYS_DIR", value_name = "DIR")]
    keys_dir: Option<String>,
    /// Not used since 0.2.0. Only here to say so.
    #[arg(long, global = true, hide = true, value_name = "FILE")]
    key_certificate: Option<String>,
    /// Setup, sign-in and password change calls a minute per address. 30 unless set.
    #[arg(long, global = true, env = "KOPPEL_RATE_AUTH_PER_MINUTE", value_name = "COUNT", value_parser = blank_ok(parse_count))]
    rate_auth_per_minute: Option<Maybe<u32>>,
    /// Skakel token requests a minute per address. 60 unless set.
    #[arg(long, global = true, env = "KOPPEL_RATE_TOKEN_PER_MINUTE", value_name = "COUNT", value_parser = blank_ok(parse_count))]
    rate_token_per_minute: Option<Maybe<u32>>,
    /// Skakel pairing calls a minute per address. 120 unless set.
    #[arg(long, global = true, env = "KOPPEL_RATE_PAIR_PER_MINUTE", value_name = "COUNT", value_parser = blank_ok(parse_count))]
    rate_pair_per_minute: Option<Maybe<u32>>,
    /// Whether discovery runs: true or false. true unless set.
    #[arg(long, global = true, env = "KOPPEL_DISCOVERY_ENABLED", value_name = "BOOL", value_parser = blank_ok(parse_bool))]
    discovery_enabled: Option<Maybe<bool>>,
    /// The UDP port for discovery. The network port unless set.
    #[arg(long, global = true, env = "KOPPEL_DISCOVERY_PORT", value_name = "PORT", value_parser = blank_ok(parse_port))]
    discovery_port: Option<Maybe<u16>>,
    /// Where sysfs is mounted. /sys unless set.
    #[arg(long, global = true, env = "KOPPEL_USB_SYS_ROOT", value_name = "DIR")]
    usb_sys_root: Option<String>,
    /// A USB/IP server (host:port) to share from instead of this kernel, such as usbipd-win.
    #[arg(
        long,
        global = true,
        env = "KOPPEL_USB_UPSTREAM",
        value_name = "HOST:PORT"
    )]
    usb_upstream: Option<String>,
    /// Windows: usbip-win2's usbip.exe. The copy the installer bundles unless set.
    #[arg(
        long,
        global = true,
        env = "KOPPEL_USB_USBIP_PATH",
        value_name = "FILE"
    )]
    usb_usbip_path: Option<String>,
    /// Seconds an attach may take before it fails. 20 unless set.
    #[arg(long, global = true, env = "KOPPEL_USB_ATTACH_TIMEOUT_SECONDS", value_name = "SECONDS", value_parser = blank_ok(parse_count))]
    usb_attach_timeout_seconds: Option<Maybe<u32>>,
    /// The port of the plain USB/IP gateway, 0 for none. Off unless set (3242 on native Windows).
    #[arg(long, global = true, env = "KOPPEL_GATEWAY_PORT", value_name = "PORT", value_parser = blank_ok(parse_gateway_port))]
    gateway_port: Option<Maybe<u16>>,
    /// The address the gateway listens on. 127.0.0.1 unless set.
    #[arg(
        long,
        global = true,
        env = "KOPPEL_GATEWAY_ADDRESS",
        value_name = "ADDRESS"
    )]
    gateway_address: Option<String>,
    /// How many gateway sessions may run at once.
    #[arg(long, global = true, env = "KOPPEL_GATEWAY_MAX_SESSIONS", value_name = "COUNT", value_parser = blank_ok(parse_count))]
    gateway_max_sessions: Option<Maybe<u32>>,
    /// Set to development to make the UI local only, as 0.1.x's ASPNETCORE_ENVIRONMENT did.
    #[arg(long, global = true, env = "KOPPEL_ENVIRONMENT", value_name = "NAME")]
    environment: Option<String>,
}

/// A value that may be blank, which counts as not set (a compose file's `KOPPEL_UI_PORT=` leaves the
/// setting alone, as a blank value did in 0.1.x).
#[derive(Debug, Clone, Copy)]
struct Maybe<T>(Option<T>);

fn blank_ok<T: Clone + Send + Sync + 'static>(
    parse: fn(&str) -> Result<T, String>,
) -> impl Fn(&str) -> Result<Maybe<T>, String> + Clone {
    move |value| {
        if value.trim().is_empty() {
            Ok(Maybe(None))
        } else {
            parse(value).map(|parsed| Maybe(Some(parsed)))
        }
    }
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| format!("\"{value}\" is not a port. Use a number from 1 to 65535."))
}

fn parse_gateway_port(value: &str) -> Result<u16, String> {
    if value.trim() == "0" {
        return Ok(0);
    }
    parse_port(value)
}

fn parse_count(value: &str) -> Result<u32, String> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| format!("\"{value}\" is not a count. Use a whole number of 1 or more."))
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("\"{value}\" is not true or false.")),
    }
}

fn parse_address(value: &str) -> Result<IpAddr, String> {
    value.trim().parse().map_err(|_| {
        format!(
            "\"{value}\" is not an IP address. Use an address such as 127.0.0.1, ::1 or 0.0.0.0."
        )
    })
}

fn parse_minutes(value: &str) -> Result<u64, String> {
    let trimmed = value.trim();
    trimmed
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| trimmed.parse::<u64>().ok())
        .flatten()
        .filter(|minutes| (1..=24 * 60).contains(minutes))
        .ok_or_else(|| {
            format!("\"{value}\" is not a number of minutes. Use a whole number from 1 to 1440.")
        })
}

fn parse_reach(value: &str) -> Result<UiReach, String> {
    UiReach::resolve(Some(value), false)
        .map_err(|e| e.replace(&format!("{} is ", UiReach::SETTING), "It is "))
}

impl Settings {
    /// The UI's port.
    pub fn ui_port(&self) -> u16 {
        set(self.ui_port).unwrap_or(crate::UI_PORT)
    }

    /// The network surface's port.
    pub fn network_port(&self) -> u16 {
        set(self.network_port).unwrap_or_else(|| self.ui_port())
    }

    /// The address the web ports bind to, `None` for every address. Other Koppel machines must reach
    /// the network port, and who may use the UI is `ui_allow_from`'s call.
    pub fn listen_address(&self) -> Option<IpAddr> {
        set(self.listen_address)
    }

    /// Who may use the UI: `lan` unless set, `local` in development.
    pub fn reach(&self) -> UiReach {
        set(self.ui_allow_from).unwrap_or(if self.development() {
            UiReach::Local
        } else {
            UiReach::Lan
        })
    }

    /// How long a machine without a UI password has for its first one. 10 minutes unless set.
    pub fn setup_window(&self) -> Duration {
        Duration::from_secs(set(self.ui_setup_window_minutes).unwrap_or(10) * 60)
    }

    /// `development`, in any case, as `ASPNETCORE_ENVIRONMENT` or `DOTNET_ENVIRONMENT` said it for 0.1.x.
    pub fn development(&self) -> bool {
        self.environment
            .as_deref()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("development"))
    }

    pub fn auth_per_minute(&self) -> u32 {
        set(self.rate_auth_per_minute).unwrap_or(30)
    }

    pub fn token_per_minute(&self) -> u32 {
        set(self.rate_token_per_minute).unwrap_or(60)
    }

    pub fn pair_per_minute(&self) -> u32 {
        set(self.rate_pair_per_minute).unwrap_or(120)
    }

    pub fn discovery_enabled(&self) -> bool {
        set(self.discovery_enabled).unwrap_or(true)
    }

    /// The UDP port for discovery, the network port unless set.
    pub fn discovery_port(&self) -> u16 {
        set(self.discovery_port).unwrap_or_else(|| self.network_port())
    }

    /// Where sysfs is mounted.
    pub fn usb_sys_root(&self) -> &str {
        non_blank(&self.usb_sys_root).unwrap_or("/sys")
    }

    /// A USB/IP server to share from, `host:port`.
    pub fn usb_upstream(&self) -> Option<&str> {
        non_blank(&self.usb_upstream)
    }

    /// usbip-win2's `usbip.exe`, when set.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn usb_usbip_path(&self) -> Option<&str> {
        non_blank(&self.usb_usbip_path)
    }

    /// Seconds an attach may take.
    pub fn usb_attach_timeout_seconds(&self) -> u32 {
        set(self.usb_attach_timeout_seconds).unwrap_or(20)
    }

    /// The gateway's port, `Some(0)` for none, `None` for the default of the crate.
    pub fn gateway_port(&self) -> Option<u16> {
        set(self.gateway_port)
    }

    pub fn gateway_address(&self) -> &str {
        non_blank(&self.gateway_address).unwrap_or("127.0.0.1")
    }

    pub fn gateway_max_sessions(&self) -> Option<u32> {
        set(self.gateway_max_sessions)
    }

    /// The config and keys folders, relative ones from `content_root`.
    pub fn paths(&self, content_root: &Path) -> KoppelPaths {
        KoppelPaths::resolve(
            non_blank(&self.config_dir),
            non_blank(&self.keys_dir),
            non_blank(&self.key_certificate),
            content_root,
        )
    }
}

fn set<T: Copy>(value: Option<Maybe<T>>) -> Option<T> {
    value.and_then(|maybe| maybe.0)
}

fn non_blank(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// What a start found: the settings, the command, and what to say about how they were spelt.
#[derive(Clone)]
pub struct Loaded {
    pub settings: Settings,
    pub command: Command,
    /// Deprecated and unknown names, to log once the log is up (see [`Loaded::log_notes`]).
    notes: Vec<String>,
}

impl Loaded {
    /// Reads the process's environment and command line. `--help` and `--version` print and exit 0,
    /// and anything unusable prints what is wrong and exits 2, before anything starts.
    pub fn read() -> Self {
        let args = std::env::args_os().skip(1).map(lossy).collect();
        let env = std::env::vars_os().map(|(k, v)| (lossy(k), lossy(v)));
        Self::parse(env, args).unwrap_or_else(|e| e.exit())
    }

    /// As [`Loaded::read`] for a given environment and command line, returning clap's error.
    pub fn parse(
        env: impl IntoIterator<Item = (String, String)>,
        args: Vec<String>,
    ) -> Result<Self, clap::Error> {
        let normalized = Normalized::new(env, args);
        let cli = Cli::try_parse_from(normalized.argv)?;
        Ok(Self {
            settings: cli.settings,
            command: cli.command.unwrap_or_default(),
            notes: normalized.notes,
        })
    }

    /// Logs each note as a warning, once.
    pub fn log_notes(&mut self) {
        for note in self.notes.drain(..) {
            tracing::warn!("{note}");
        }
        if self.settings.key_certificate.is_some() {
            tracing::warn!(
                "Paths:KeyCertificate is set, but Koppel 0.2 no longer reads it. It was deprecated in 0.1.2"
            );
        }
    }
}

fn lossy(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

/// The command line clap reads, and what to say about the old spellings that became it.
struct Normalized {
    argv: Vec<String>,
    notes: Vec<String>,
}

impl Normalized {
    fn new(env: impl IntoIterator<Item = (String, String)>, args: Vec<String>) -> Self {
        let env: Vec<(String, String)> = env.into_iter().collect();
        let mut notes = Vec::new();
        let mut argv = vec!["koppel".to_owned()];

        // Old environment names become options, ahead of the real ones so those win, unless the new
        // name is set as well, which then wins over the old.
        let known = known_env_names();
        for (name, value) in &env {
            if let Some(long) = legacy_long(&legacy_env_key(name)) {
                let new = env_name(long);
                let new_set = env.iter().any(|(n, v)| *n == new && !v.trim().is_empty());
                if value.trim().is_empty() {
                    continue;
                }
                if new_set {
                    notes.push(format!(
                        "{name} is ignored: {new} is set, and it replaces {name}, which is deprecated"
                    ));
                } else {
                    notes.push(format!("{name} is deprecated: use {new}"));
                    argv.push(format!("--{long}={value}"));
                }
            } else if let Some(note) = unknown_env(name, &known) {
                notes.push(note);
            }
        }

        let mut args = args.into_iter().peekable();
        let mut command = None;
        while let Some(arg) = args.next() {
            if let Some((word, _)) = Command::from_flag(&arg) {
                notes.push(format!("{arg} is deprecated: use the command {word}"));
                command = Some(word);
                continue;
            }
            match legacy_option(&arg) {
                Some((name, inline)) => match legacy_long(&name) {
                    Some(long) => {
                        let value = inline.or_else(|| args.next_if(|next| !next.starts_with("--")));
                        notes.push(format!(
                            "{arg} is deprecated: use --{long} (or {})",
                            env_name(long)
                        ));
                        if let Some(value) = value {
                            argv.push(format!("--{long}={value}"));
                        } else {
                            argv.push(format!("--{long}"));
                        }
                    }
                    None => notes.push(format!("{arg} is not a Koppel setting and is ignored")),
                },
                None => argv.push(arg),
            }
        }
        // A command given as the old flag goes first, where a command belongs.
        if let Some(word) = command {
            argv.insert(1, word.to_owned());
        }
        Self { argv, notes }
    }
}

/// The env var name of an option: `KOPPEL_` and the long name in capitals.
fn env_name(long: &str) -> String {
    format!("KOPPEL_{}", long.to_ascii_uppercase().replace('-', "_"))
}

/// An environment variable's name as a legacy key: `__` read as `:`, lower case.
fn legacy_env_key(name: &str) -> String {
    name.replace("__", ":").to_ascii_lowercase()
}

/// The option for a legacy key (lower case, `:` between section and key).
fn legacy_long(key: &str) -> Option<&'static str> {
    LEGACY
        .iter()
        .find(|(_, legacy)| legacy.to_ascii_lowercase() == key)
        .map(|(long, _)| *long)
}

/// A command line argument in 0.1.x's form, `--Ports:Ui=1`, `--Ports:Ui 1`, `/Ports:Ui=1` or
/// `Ports:Ui=1`: its legacy key and the value after `=`, if any. Anything else is not one. A name
/// counts when it has a `:` or `__` in it, or is a known legacy name such as `ASPNETCORE_ENVIRONMENT`.
fn legacy_option(arg: &str) -> Option<(String, Option<String>)> {
    let bare = arg
        .strip_prefix("--")
        .or_else(|| arg.strip_prefix('/'))
        .unwrap_or(arg);
    let (name, value) = match bare.split_once('=') {
        Some((name, value)) => (name, Some(value.to_owned())),
        None => (bare, None),
    };
    let shaped = is_section_key(&name.replace("__", ":"), ':');
    let prefixed = bare.len() != arg.len();
    let known = legacy_long(&legacy_env_key(name)).is_some();
    // Without a prefix, only `Section:Key=value` is one, so a path or a word is left to clap.
    ((prefixed && (shaped || known)) || (!prefixed && shaped && value.is_some()))
        .then(|| (legacy_env_key(name), value))
}

/// `Left<sep>Right`, both letters and digits.
fn is_section_key(name: &str, sep: char) -> bool {
    name.split_once(sep).is_some_and(|(section, key)| {
        !section.is_empty()
            && !key.is_empty()
            && section.chars().all(|c| c.is_ascii_alphanumeric())
            && key.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// The `KOPPEL_` names clap reads, from the options themselves.
fn known_env_names() -> Vec<String> {
    Cli::command()
        .get_arguments()
        .filter_map(|arg| arg.get_env())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

/// What to say about an environment variable that looks like Koppel's but is not, or `None`.
fn unknown_env(name: &str, known: &[String]) -> Option<String> {
    if name.to_ascii_uppercase().starts_with("KOPPEL_") {
        let upper = name.to_ascii_uppercase();
        if known.contains(&name.to_owned())
            || OTHER_KOPPEL_NAMES.contains(&upper.as_str())
            || OTHER_KOPPEL_PREFIXES.iter().any(|p| upper.starts_with(p))
        {
            return None;
        }
        return Some(unknown_note(name, &upper, known.iter().map(String::as_str)));
    }
    // Section__Key, as 0.1.x read it. Anything else with `__` in it is not ours to judge.
    if !is_section_key(&name.replace("__", ":"), ':') {
        return None;
    }
    let legacy: Vec<String> = LEGACY.iter().map(|(_, l)| l.replace(':', "__")).collect();
    Some(unknown_note(
        name,
        &name.to_ascii_lowercase(),
        legacy.iter().map(String::as_str),
    ))
}

fn unknown_note<'a>(name: &str, probe: &str, candidates: impl Iterator<Item = &'a str>) -> String {
    let best = candidates
        .map(|c| {
            (
                distance(&probe.to_ascii_lowercase(), &c.to_ascii_lowercase()),
                c,
            )
        })
        .min_by_key(|(d, _)| *d);
    match best {
        Some((d, c)) if d <= 3 => {
            format!("{name} is not a Koppel setting and is ignored. Did you mean {c}?")
        }
        _ => format!("{name} is not a Koppel setting and is ignored"),
    }
}

/// The edit distance between two short names.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(env: &[(&str, &str)], args: &[&str]) -> Result<Loaded, clap::Error> {
        Loaded::parse(
            env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
            args.iter().map(|a| (*a).to_owned()).collect(),
        )
    }

    fn ok(env: &[(&str, &str)], args: &[&str]) -> Loaded {
        load(env, args).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn the_old_environment_names_still_work_in_any_case() {
        let l = ok(
            &[
                ("Paths__Config", "/data"),
                ("UI__ALLOWFROM", "local"),
                ("Ports__Ui", "15240"),
            ],
            &[],
        );
        assert_eq!(l.settings.config_dir.as_deref(), Some("/data"));
        assert_eq!(l.settings.reach(), UiReach::Local);
        assert_eq!(l.settings.ui_port(), 15240);
        assert_eq!(l.command, Command::Serve);
        assert!(
            l.notes
                .iter()
                .any(|n| n.contains("Ports__Ui is deprecated: use KOPPEL_UI_PORT"))
        );
    }

    #[test]
    fn the_old_command_line_forms_still_work_and_win_over_the_environment() {
        let l = ok(
            &[("Ports__Ui", "1")],
            &[
                "--Ports:Ui=15240",
                "--Ports:Network",
                "15241",
                "Ui:SetupWindowMinutes=2",
                "/Discovery:Enabled=false",
            ],
        );
        assert_eq!(l.settings.ui_port(), 15240);
        assert_eq!(l.settings.network_port(), 15241);
        assert_eq!(l.settings.setup_window(), Duration::from_secs(120));
        assert!(!l.settings.discovery_enabled());
        assert!(
            l.notes
                .iter()
                .any(|n| n.contains("--Ports:Ui=15240 is deprecated: use --ui-port"))
        );
    }

    #[test]
    fn a_new_option_wins_over_an_old_name_and_a_new_variable_over_an_old_one() {
        let l = ok(&[("Ports__Ui", "1")], &["--ui-port", "15240"]);
        assert_eq!(l.settings.ui_port(), 15240);
        // clap reads KOPPEL_UI_PORT itself, from the process; here only the old name's note shows.
        let l = ok(&[("Ports__Ui", "1"), ("KOPPEL_UI_PORT", "15240")], &[]);
        assert_eq!(set(l.settings.ui_port), None);
        assert!(l.notes.iter().any(|n| n.contains("Ports__Ui is ignored")));
    }

    #[test]
    fn the_old_flags_are_the_commands() {
        let l = ok(&[], &["--reset-password"]);
        assert_eq!(l.command, Command::ResetPassword);
        assert!(
            l.notes
                .iter()
                .any(|n| n.contains("--reset-password is deprecated"))
        );
        assert_eq!(ok(&[], &["--HealthCheck"]).command, Command::Healthcheck);
        assert_eq!(ok(&[], &["healthcheck"]).command, Command::Healthcheck);
        assert_eq!(
            ok(&[], &["prepare-data-folder", "--check", "some/data"]).command,
            Command::PrepareDataFolder {
                check: true,
                path: Some("some/data".to_owned())
            }
        );
        assert_eq!(
            ok(&[], &["prepare-data-folder"]).command,
            Command::PrepareDataFolder {
                check: false,
                path: None
            }
        );
        assert_eq!(ok(&[], &["serve"]).command, Command::Serve);
        assert_eq!(ok(&[], &[]).command, Command::Serve);
    }

    #[test]
    fn options_come_before_or_after_a_command() {
        assert_eq!(ok(&[], &["serve", "--ui-port=7"]).settings.ui_port(), 7);
        assert_eq!(
            ok(&[], &["--ui-port=7", "healthcheck"]).settings.ui_port(),
            7
        );
        assert_eq!(
            ok(&[], &["healthcheck", "--Ports:Ui=8"]).settings.ui_port(),
            8
        );
    }

    #[test]
    fn a_bad_value_names_its_option() {
        for (args, name) in [
            (&["--ui-port=x"][..], "--ui-port"),
            (&["--listen-address=lan"], "--listen-address"),
            (
                &["--ui-setup-window-minutes=0"],
                "--ui-setup-window-minutes",
            ),
            (&["--rate-auth-per-minute=-1"], "--rate-auth-per-minute"),
            (&["--ui-allow-from=everyone"], "--ui-allow-from"),
            (&["--discovery-enabled=maybe"], "--discovery-enabled"),
        ] {
            let e = load(&[], args)
                .err()
                .unwrap_or_else(|| panic!("{args:?} was accepted"));
            assert!(e.to_string().contains(name), "{e}");
        }
        let e = load(&[("Ports__Ui", "x")], &[])
            .err()
            .expect("a bad old name");
        assert!(e.to_string().contains("--ui-port"), "{e}");
    }

    #[test]
    fn an_unknown_option_is_refused_and_an_unknown_setting_is_warned_about() {
        assert!(load(&[], &["--no-such-thing"]).is_err());
        let l = ok(
            &[("Ports__Uii", "5"), ("KOPPEL_UI_PROT", "5")],
            &["--Ports:Uii=1"],
        );
        let all = l.notes.join("\n");
        assert!(
            all.contains(
                "Ports__Uii is not a Koppel setting and is ignored. Did you mean Ports__Ui?"
            ),
            "{all}"
        );
        assert!(all.contains("KOPPEL_UI_PROT is not a Koppel setting and is ignored. Did you mean KOPPEL_UI_PORT?"), "{all}");
        assert!(
            all.contains("--Ports:Uii=1 is not a Koppel setting"),
            "{all}"
        );
        assert_eq!(set(l.settings.ui_port), None);
    }

    #[test]
    fn names_that_are_not_settings_are_not_warned_about() {
        let l = ok(
            &[
                ("KOPPEL_RUNNING_IN_CONTAINER", "true"),
                ("KOPPEL_CONTRACT_URL", "x"),
                ("PATH", "x"),
                ("__COMPAT_LAYER", "x"),
                ("A_B__C_D", "x"),
                ("RUST_LOG", "debug"),
            ],
            &[],
        );
        assert!(l.notes.is_empty(), "{:?}", l.notes);
    }

    #[test]
    fn development_makes_the_ui_local_and_blank_values_are_unset() {
        assert_eq!(
            ok(&[("ASPNETCORE_ENVIRONMENT", "Development")], &[])
                .settings
                .reach(),
            UiReach::Local
        );
        assert_eq!(ok(&[], &[]).settings.reach(), UiReach::Lan);
        assert_eq!(ok(&[("Ports__Ui", "  ")], &[]).settings.ui_port(), 5240);
    }

    #[test]
    fn defaults_follow_one_another() {
        let s = ok(&[], &["--ui-port=1000"]).settings;
        assert_eq!((s.network_port(), s.discovery_port()), (1000, 1000));
        let s = ok(&[], &["--ui-port=1000", "--network-port=1001"]).settings;
        assert_eq!(s.discovery_port(), 1001);
        assert_eq!(
            ok(&[], &["--gateway-port=0"]).settings.gateway_port(),
            Some(0)
        );
    }

    #[test]
    fn every_old_name_has_a_new_variable_and_the_other_way_round() {
        let command = Cli::command();
        let with_env: Vec<_> = command
            .get_arguments()
            .filter_map(|a| {
                a.get_env().map(|e| {
                    (
                        a.get_id().as_str().to_owned(),
                        e.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        for (id, env) in &with_env {
            assert_eq!(*env, env_name(&id.replace('_', "-")));
            assert!(
                LEGACY.iter().any(|(long, _)| long.replace('-', "_") == *id),
                "{id} has no old name"
            );
        }
        for (long, legacy) in LEGACY {
            let id = long.replace('-', "_");
            assert!(
                command.get_arguments().any(|a| a.get_id() == id.as_str()),
                "{legacy}"
            );
        }
    }
}
