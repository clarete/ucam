use serde_derive::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ConfigUserAuth {
    pub(crate) token_validity: u64,
    pub(crate) jwt_secret: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ConfigHTTP {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) key: String,
    pub(crate) cert: String,
    pub(crate) cacert: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) http: ConfigHTTP,
    pub(crate) userauth: ConfigUserAuth,
}
