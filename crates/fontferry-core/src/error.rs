use thiserror::Error;

#[derive(Debug, Error)]
pub enum FontFerryError {
    #[error("invalid catalog entry: {0}")]
    InvalidCatalog(String),
    #[error("unknown font: {0}")]
    UnknownFont(String),
    #[error("no eligible version is available")]
    NoEligibleVersion,
    #[error("license acceptance is required")]
    LicenseAcceptanceRequired,
    #[error("this font is reminder-only and cannot be downloaded")]
    ReminderOnly,
    #[error("no rollback snapshot is available")]
    NoRollbackSnapshot,
    #[error("download rejected: {0}")]
    DownloadRejected(String),
    #[error("archive rejected: {0}")]
    ArchiveRejected(String),
    #[error("font rejected: {0}")]
    FontRejected(String),
    #[error("catalog signature verification failed")]
    InvalidCatalogSignature,
    #[error("platform operation failed: {0}")]
    Platform(String),
    #[error("state operation failed: {0}")]
    State(String),
    #[error("network operation failed: {0}")]
    Network(String),
}

pub type Result<T> = std::result::Result<T, FontFerryError>;
