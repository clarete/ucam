use std::io;

#[derive(Debug)]
pub(crate) struct Error {}

impl Error {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Error")
    }
}

impl From<base64::DecodeError> for Error {
    fn from(_err: base64::DecodeError) -> Self {
        Error::new()
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(_err: std::str::Utf8Error) -> Self {
        Error::new()
    }
}

impl From<std::time::SystemTimeError> for Error {
    fn from(_err: std::time::SystemTimeError) -> Self {
        Error::new()
    }
}

impl From<jsonwebtoken::errors::Error> for Error {
    fn from(_err: jsonwebtoken::errors::Error) -> Self {
        Error::new()
    }
}

impl From<actix::MailboxError> for Error {
    fn from(_err: actix::MailboxError) -> Self {
        Error::new()
    }
}

impl actix_web::ResponseError for Error {
}

/// Application error types.
#[derive(Debug)]
pub(crate) enum ChatError {
    IO(io::Error),
    Config(toml::de::Error),
    // Web(actix_web::Error),
}

impl From<io::Error> for ChatError {
    fn from(error: io::Error) -> Self {
        ChatError::IO(error)
    }
}

impl From<toml::de::Error> for ChatError {
    fn from(error: toml::de::Error) -> Self {
        ChatError::Config(error)
    }
}

impl From<ChatError> for io::Error {
    fn from(error: ChatError) -> Self {
        match error {
            ChatError::IO(e) => e,
            _ => io::Error::from(error),
        }
    }
}
