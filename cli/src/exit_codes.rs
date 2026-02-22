use fuelcheck_core::errors::CliError;
use fuelcheck_core::model::ErrorKind;

pub fn exit_code_for_error(err: &anyhow::Error) -> i32 {
    if let Some(cli_err) = err.downcast_ref::<CliError>() {
        return match cli_err {
            CliError::UnknownProvider(_) => 2,
            CliError::ProviderNotImplemented(_) => 2,
            CliError::ConfigMissing(_) | CliError::ConfigPathUnavailable => 3,
            CliError::ProviderNotConfigured(_) => 2,
            CliError::UnsupportedSource(_, _) => 3,
        };
    }
    if let Some(req_err) = err.downcast_ref::<reqwest::Error>()
        && req_err.is_timeout()
    {
        return 4;
    }
    if err.downcast_ref::<tokio::time::error::Elapsed>().is_some() {
        return 4;
    }
    if err.downcast_ref::<serde_json::Error>().is_some() {
        return 3;
    }
    1
}

pub fn error_kind_for_error(err: &anyhow::Error) -> ErrorKind {
    if let Some(cli_err) = err.downcast_ref::<CliError>() {
        return match cli_err {
            CliError::UnknownProvider(_) => ErrorKind::Args,
            CliError::ProviderNotImplemented(_) => ErrorKind::Provider,
            CliError::ConfigMissing(_) | CliError::ConfigPathUnavailable => ErrorKind::Config,
            CliError::ProviderNotConfigured(_) => ErrorKind::Provider,
            CliError::UnsupportedSource(_, _) => ErrorKind::Args,
        };
    }
    ErrorKind::Runtime
}
