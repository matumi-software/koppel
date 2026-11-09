//! Koppel's settings, read where 0.1.x read them: environment variables, with `__` for the `:` of a
//! section (`Paths__Config`, `Ui__AllowFrom`), and the command line, `--Paths:Config=/data` or
//! `--Paths:Config /data`, which wins. Names are matched without regard to case, as .NET's
//! configuration does. 0.1.x's `appsettings.json` held only log levels, so there is no file to read.

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use koppel_config::KoppelPaths;
use koppel_web::UiReach;

/// The commands that take no value and are not settings.
pub const FLAGS: &[&str] = &["--reset-password"];

/// Every setting, by its name in lower case with `:` between sections.
#[derive(Debug, Default)]
pub struct Config {
    values: HashMap<String, String>,
}

impl Config {
    /// The environment, then the command line over it.
    pub fn read(
        environment: impl IntoIterator<Item = (OsString, OsString)>,
        args: &[String],
    ) -> Self {
        let mut values = HashMap::new();
        for (name, value) in environment {
            if let (Some(name), Some(value)) = (name.to_str(), value.to_str()) {
                values.insert(
                    name.replace("__", ":").to_ascii_lowercase(),
                    value.to_owned(),
                );
            }
        }
        let mut args = args.iter().peekable();
        while let Some(arg) = args.next() {
            if FLAGS.iter().any(|flag| arg.eq_ignore_ascii_case(flag)) {
                continue;
            }
            let bare = arg
                .strip_prefix("--")
                .or_else(|| arg.strip_prefix('/'))
                .unwrap_or(arg);
            if let Some((name, value)) = bare.split_once('=') {
                values.insert(
                    name.replace("__", ":").to_ascii_lowercase(),
                    value.to_owned(),
                );
            } else if (arg.starts_with("--") || arg.starts_with('/'))
                && let Some(value) = args.next_if(|next| !next.starts_with("--"))
            {
                values.insert(bare.replace("__", ":").to_ascii_lowercase(), value.clone());
            }
        }
        Self { values }
    }

    /// A setting's value, `None` when it is unset or empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    }

    /// A port number, `default` when unset. Anything else stops the start, naming the setting.
    pub fn port(&self, name: &str, default: u16) -> Result<u16, String> {
        match self.get(name) {
            None => Ok(default),
            Some(value) => value
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| {
                    format!("{name} is \"{value}\". Use a port number from 1 to 65535.")
                }),
        }
    }

    /// A count of at least one, `default` when unset.
    pub fn count(&self, name: &str, default: u32) -> Result<u32, String> {
        match self.get(name) {
            None => Ok(default),
            Some(value) => value
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|count| *count > 0)
                .ok_or_else(|| format!("{name} is \"{value}\". Use a whole number of 1 or more.")),
        }
    }

    /// `true` or `false` in any case, as .NET reads a bool, `default` when unset.
    pub fn flag(&self, name: &str, default: bool) -> Result<bool, String> {
        match self.get(name).map(str::trim) {
            None => Ok(default),
            Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
            Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
            Some(value) => Err(format!("{name} is \"{value}\". Use true or false.")),
        }
    }

    /// `Ports:Address`: an IP address, or `None` for every address. Other Koppel machines must reach
    /// the network port, and who may use the UI is `Ui:AllowFrom`'s call.
    pub fn listen_address(&self) -> Result<Option<IpAddr>, String> {
        const NAME: &str = "Ports:Address";
        match self.get(NAME) {
            None => Ok(None),
            Some(value) => value.trim().parse().map(Some).map_err(|_| {
                format!(
                    "{NAME} is \"{value}\", which is not an IP address. Use an address such as 127.0.0.1, ::1 or 0.0.0.0."
                )
            }),
        }
    }

    /// `Ui:SetupWindowMinutes`: whole minutes from 1 to 1440, 10 when unset.
    pub fn setup_window(&self) -> Result<Duration, String> {
        const NAME: &str = "Ui:SetupWindowMinutes";
        match self.get(NAME) {
            None => Ok(Duration::from_secs(10 * 60)),
            Some(value) => {
                let trimmed = value.trim();
                trimmed
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                    .then(|| trimmed.parse::<u64>().ok())
                    .flatten()
                    .filter(|minutes| (1..=24 * 60).contains(minutes))
                    .map(|minutes| Duration::from_secs(minutes * 60))
                    .ok_or_else(|| {
                        format!(
                            "{NAME} is \"{value}\". Use a whole number of minutes from 1 to 1440."
                        )
                    })
            }
        }
    }

    /// `Ui:AllowFrom`: `lan` unless set, `local` in Development.
    pub fn reach(&self) -> Result<UiReach, String> {
        UiReach::resolve(self.get(UiReach::SETTING), self.development())
    }

    /// `ASPNETCORE_ENVIRONMENT` or `DOTNET_ENVIRONMENT` set to Development, as for 0.1.x.
    pub fn development(&self) -> bool {
        ["ASPNETCORE_ENVIRONMENT", "DOTNET_ENVIRONMENT"]
            .iter()
            .any(|name| {
                self.get(name)
                    .is_some_and(|value| value.eq_ignore_ascii_case("development"))
            })
    }

    /// `Paths:Config` and `Paths:Keys`, relative ones from `content_root`, as `KoppelPaths.cs` read
    /// them.
    pub fn paths(&self, content_root: &Path) -> KoppelPaths {
        KoppelPaths::resolve(
            self.get("Paths:Config"),
            self.get("Paths:Keys"),
            self.get("Paths:KeyCertificate"),
            content_root,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(env: &[(&str, &str)], args: &[&str]) -> Config {
        Config::read(
            env.iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v))),
            &args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn the_environment_and_the_command_line_are_read_as_dotnet_reads_them() {
        let c = config(
            &[
                ("Paths__Config", "/data"),
                ("UI__ALLOWFROM", "local"),
                ("Ports__Ui", "1"),
            ],
            &[
                "--Ports:Ui=15240",
                "--reset-password",
                "--Ports:Network",
                "15241",
                "Ui:SetupWindowMinutes=2",
            ],
        );
        assert_eq!(c.get("paths:config"), Some("/data"));
        assert_eq!(c.reach(), Ok(UiReach::Local));
        assert_eq!(c.port("Ports:Ui", 5240), Ok(15240));
        assert_eq!(c.port("Ports:Network", 5240), Ok(15241));
        assert_eq!(c.setup_window(), Ok(Duration::from_secs(120)));
        assert_eq!(c.port("Ports:Missing", 5240), Ok(5240));
    }

    #[test]
    fn a_bad_value_names_its_setting() {
        let c = config(
            &[
                ("Ports__Ui", "x"),
                ("Ports__Address", "lan"),
                ("Ui__SetupWindowMinutes", "0"),
                ("RateLimits__AuthPerMinute", "-1"),
            ],
            &[],
        );
        assert!(c.port("Ports:Ui", 5240).unwrap_err().contains("Ports:Ui"));
        assert!(c.listen_address().unwrap_err().contains("Ports:Address"));
        assert!(
            c.setup_window()
                .unwrap_err()
                .contains("Ui:SetupWindowMinutes")
        );
        assert!(c.count("RateLimits:AuthPerMinute", 30).is_err());
    }

    #[test]
    fn development_makes_the_ui_local() {
        assert_eq!(
            config(&[("ASPNETCORE_ENVIRONMENT", "Development")], &[]).reach(),
            Ok(UiReach::Local)
        );
        assert_eq!(config(&[], &[]).reach(), Ok(UiReach::Lan));
    }
}
