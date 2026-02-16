use crate::providers::ProviderId;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("config path unavailable (no home directory)")]
    ConfigPathUnavailable,
    #[error("config file missing: {0}")]
    ConfigMissing(PathBuf),
    #[error("provider {0} not configured")]
    ProviderNotConfigured(ProviderId),
    #[error("provider {0} does not support source {1}")]
    UnsupportedSource(ProviderId, String),
    #[error("provider {0} not implemented yet")]
    ProviderNotImplemented(ProviderId),
}
