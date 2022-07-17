#[derive(Debug)]
pub(crate) enum ErrorType {
    IO,
    Input,
    Gst,
    Protocol,
}

#[derive(Debug)]
pub(crate) struct Error {
    t: ErrorType,
    message: String,
}

impl Error {
    pub(crate) fn new(t: ErrorType, message: String) -> Self {
        Self { t, message }
    }

    pub(crate) fn new_io(message: String) -> Self {
        Error::new(ErrorType::IO, message)
    }

    pub(crate) fn new_input(message: String) -> Self {
        Error::new(ErrorType::Input, message)
    }

    pub(crate) fn new_gst(message: String) -> Self {
        Error::new(ErrorType::Gst, message)
    }

    pub(crate) fn new_proto(message: String) -> Self {
        Error::new(ErrorType::Protocol, message)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?} Error: {}", self.t, self.message)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::new_io(err.to_string())
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(err: std::str::Utf8Error) -> Self {
        Error::new_input(err.to_string())
    }
}

impl From<openssl::error::ErrorStack> for Error {
    fn from(err: openssl::error::ErrorStack) -> Self {
        Error::new_io(err.to_string())
    }
}

impl From<gst::glib::Error> for Error {
    fn from(err: gst::glib::Error) -> Self {
        Error::new_gst(err.to_string())
    }
}

impl From<gst::glib::BoolError> for Error {
    fn from(err: gst::glib::BoolError) -> Self {
        Error::new_gst(err.to_string())
    }
}

impl From<gst::PadLinkError> for Error {
    fn from(err: gst::PadLinkError) -> Self {
        Error::new_gst(err.to_string())
    }
}

impl From<gst::structure::GetError<'_>> for Error {
    fn from(err: gst::structure::GetError<'_>) -> Self {
        Error::new_gst(err.to_string())
    }
}

impl From<toml::de::Error> for Error {
    fn from(err: toml::de::Error) -> Self {
        Error::new_input(err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::new_input(err.to_string())
    }
}

impl From<futures::channel::mpsc::TrySendError<protocol::Envelope>> for Error {
    fn from(err: futures::channel::mpsc::TrySendError<protocol::Envelope>) -> Self {
        Error::new_input(err.to_string())
    }
}
