use chrono::Utc;
use clap::ValueEnum;
use serde_json::json;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogLevel {
    Trace,
    Verbose,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl LogLevel {
    fn priority(self) -> u8 {
        match self {
            LogLevel::Trace => 0,
            LogLevel::Verbose => 1,
            LogLevel::Debug => 2,
            LogLevel::Info => 3,
            LogLevel::Warning => 4,
            LogLevel::Error => 5,
            LogLevel::Critical => 6,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Verbose => "verbose",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warning => "warning",
            LogLevel::Error => "error",
            LogLevel::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoggerConfig {
    pub level: LogLevel,
    pub json_output: bool,
    pub json_only: bool,
}

static LOGGER: OnceLock<LoggerConfig> = OnceLock::new();

pub fn init(config: LoggerConfig) {
    let _ = LOGGER.set(config);
}

pub fn log(
    level: LogLevel,
    event: &str,
    message: impl AsRef<str>,
    context: Option<serde_json::Value>,
) {
    let Some(config) = LOGGER.get() else {
        return;
    };
    if level.priority() < config.level.priority() {
        return;
    }
    if config.json_output {
        let payload = json!({
            "ts": Utc::now().to_rfc3339(),
            "level": level.as_str(),
            "event": event,
            "message": message.as_ref(),
            "context": context,
        });
        if let Ok(line) = serde_json::to_string(&payload) {
            eprintln!("{}", line);
        }
        return;
    }

    if config.json_only {
        return;
    }

    eprintln!("[{}] {}: {}", level.as_str(), event, message.as_ref());
}
