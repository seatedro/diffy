use keyring::Entry;

use crate::core::error::{DiffyError, Result};

const SERVICE: &str = "diffy";
const ACCOUNT: &str = "github.access_token";

fn entry() -> Result<Entry> {
    Entry::new(SERVICE, ACCOUNT).map_err(to_diffy_error)
}

fn to_diffy_error(error: keyring::Error) -> DiffyError {
    DiffyError::General(format!("keyring: {error}"))
}

pub fn load_github_token() -> Result<Option<String>> {
    match entry()?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(to_diffy_error(error)),
    }
}

pub fn save_github_token(token: &str) -> Result<()> {
    entry()?.set_password(token).map_err(to_diffy_error)
}

pub fn clear_github_token() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(to_diffy_error(error)),
    }
}
