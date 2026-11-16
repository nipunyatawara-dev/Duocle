use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const HELP: &str = "Duocle commands:
  status | on | off | toggle | peek | settings | quit
  mode ambient|deep
  ignore
  set KEY JSON_VALUE

Examples:
  duocle set blur 65
  duocle set app_wide true
  duocle set tint '\"#233550\"'";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "action", content = "value")]
pub enum Command {
    Status,
    On,
    Off,
    Toggle,
    Peek,
    Settings,
    Quit,
    Mode(crate::config::Mode),
    Ignore,
    Set(String, Value),
}

#[derive(Deserialize, Serialize)]
struct Request {
    expires: u64,
    command: Command,
}

#[derive(Deserialize, Serialize)]
pub struct Response {
    pub result: Value,
    pub error: Option<String>,
}

pub struct Paths {
    pub settings: PathBuf,
    ipc: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let project = directories::ProjectDirs::from("app", "", "Duocle")
            .context("no user configuration directory")?;
        let root = project.config_dir();
        let ipc = project.runtime_dir().unwrap_or(root).join("commands");
        make_private(root)?;
        make_private(&ipc)?;
        Ok(Self {
            settings: root.join("settings.json"),
            ipc,
        })
    }

    pub fn lock(&self) -> Result<Option<File>> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.ipc.join("instance.lock"))?;
        match FileExt::try_lock_exclusive(&lock) {
            Ok(()) => Ok(Some(lock)),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn invoke(&self, command: Command) -> Result<Value> {
        const WAIT: Duration = Duration::from_secs(4);
        if self.lock()?.is_some() {
            bail!("Duocle is not running. Start the desktop app and run the command again.");
        }
        let expires = now() + WAIT.as_millis() as u64;
        let mut file = tempfile::Builder::new()
            .prefix("request-")
            .suffix(".tmp")
            .tempfile_in(&self.ipc)?;
        file.write_all(&serde_json::to_vec(&Request { expires, command })?)?;
        let request = file.path().with_extension("json");
        let response = file.path().with_extension("reply");
        file.persist_noclobber(&request)?;
        while now() < expires {
            if let Ok(bytes) = fs::read(&response) {
                let _ = fs::remove_file(&response);
                let response: Response = serde_json::from_slice(&bytes)?;
                if let Some(error) = response.error {
                    bail!("{error}");
                }
                return Ok(response.result);
            }
            thread::sleep(Duration::from_millis(30));
        }
        let _ = fs::remove_file(&request);
        bail!("Duocle did not respond within {} seconds.", WAIT.as_secs());
    }

    pub fn pending(&self) -> Vec<(PathBuf, Command)> {
        let Ok(entries) = fs::read_dir(&self.ipc) else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_name().to_string_lossy().starts_with("request-") {
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "reply") {
                if entry
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|elapsed| elapsed > Duration::from_secs(30))
                {
                    let _ = fs::remove_file(path);
                }
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let request = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok());
            let _ = fs::remove_file(&path);
            if let Some(request) = request.filter(|request| request.expires > now()) {
                requests.push((path.with_extension("reply"), request.command));
            }
        }
        requests
    }

    pub fn respond(path: &Path, result: Result<Value>) {
        let response = match result {
            Ok(result) => Response {
                result,
                error: None,
            },
            Err(error) => Response {
                result: Value::Null,
                error: Some(format!("{error:#}")),
            },
        };
        if let Ok(bytes) = serde_json::to_vec(&response) {
            let _ = crate::config::write_atomic(path, &bytes);
        }
    }
}

fn make_private(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn parse_cli(args: &[String]) -> Result<Option<Command>> {
    let command = match args.first().map(String::as_str) {
        None | Some("--background") => return Ok(None),
        Some("-h" | "--help") => {
            println!("{HELP}");
            return Ok(None);
        }
        Some("status") if args.len() == 1 => Command::Status,
        Some("on") if args.len() == 1 => Command::On,
        Some("off") if args.len() == 1 => Command::Off,
        Some("toggle") if args.len() == 1 => Command::Toggle,
        Some("peek") if args.len() == 1 => Command::Peek,
        Some("settings") if args.len() == 1 => Command::Settings,
        Some("quit") if args.len() == 1 => Command::Quit,
        Some("ignore") if args.len() == 1 => Command::Ignore,
        Some("mode") if args.len() == 2 => Command::Mode(mode(&args[1])?),
        Some("set") if args.len() == 3 => Command::Set(
            args[1].clone(),
            serde_json::from_str(&args[2]).context("expected a JSON value")?,
        ),
        _ => bail!("{HELP}"),
    };
    Ok(Some(command))
}

pub fn parse_url(url: &str) -> Result<Command> {
    let url = url::Url::parse(url)?;
    if url.scheme() != "duocle"
        || !matches!(url.path(), "" | "/")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        bail!("Invalid Duocle URL");
    }
    let command = match url.host_str() {
        Some("on") => Command::On,
        Some("off") => Command::Off,
        Some("toggle") => Command::Toggle,
        Some("peek") => Command::Peek,
        Some("settings") => Command::Settings,
        Some("mode") => Command::Mode(mode(&parameter(&url)?)?),
        Some(key @ ("blur" | "tint_strength" | "grain" | "monochrome" | "transition")) => {
            Command::Set(key.into(), Value::from(parameter(&url)?.parse::<u16>()?))
        }
        Some("tint") => Command::Set("tint".into(), Value::from(parameter(&url)?)),
        Some("app_wide") => Command::Set(
            "app_wide".into(),
            Value::from(parameter(&url)?.parse::<bool>()?),
        ),
        Some("displays") => Command::Set("displays".into(), Value::from(parameter(&url)?)),
        _ => bail!("Unknown Duocle URL command"),
    };
    Ok(command)
}

fn parameter(url: &url::Url) -> Result<String> {
    url.query_pairs()
        .find(|(key, _)| key == "value")
        .map(|(_, value)| value.into_owned())
        .context("missing URL value")
}

fn mode(value: &str) -> Result<crate::config::Mode> {
    use crate::config::Mode;
    match value {
        "ambient" => Ok(Mode::Ambient),
        "deep" => Ok(Mode::Deep),
        _ => bail!("Expected ambient or deep"),
    }
}
