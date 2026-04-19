use keyring::Entry;

use crate::core::error::{DiffyError, Result};

const SERVICE: &str = "diffy";
const GITHUB_ACCOUNT: &str = "github.access_token";
const OPENAI_ACCOUNT: &str = "ai.openai_api_key";
const ANTHROPIC_ACCOUNT: &str = "ai.anthropic_api_key";

fn entry_for(account: &str) -> Result<Entry> {
    Entry::new(SERVICE, account).map_err(to_diffy_error)
}

fn to_diffy_error(error: keyring::Error) -> DiffyError {
    DiffyError::General(format!("keyring: {error}"))
}

fn load(account: &str) -> Result<Option<String>> {
    match entry_for(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(to_diffy_error(error)),
    }
}

fn save(account: &str, value: &str) -> Result<()> {
    entry_for(account)?
        .set_password(value)
        .map_err(to_diffy_error)
}

fn clear(account: &str) -> Result<()> {
    match entry_for(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(to_diffy_error(error)),
    }
}

pub fn load_github_token() -> Result<Option<String>> {
    load(GITHUB_ACCOUNT)
}

pub fn save_github_token(token: &str) -> Result<()> {
    save(GITHUB_ACCOUNT, token)
}

pub fn clear_github_token() -> Result<()> {
    clear(GITHUB_ACCOUNT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiKeyKind {
    OpenAi,
    Anthropic,
}

impl AiKeyKind {
    fn account(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_ACCOUNT,
            Self::Anthropic => ANTHROPIC_ACCOUNT,
        }
    }
}

pub fn load_ai_key(kind: AiKeyKind) -> Result<Option<String>> {
    load(kind.account())
}

pub fn save_ai_key(kind: AiKeyKind, value: &str) -> Result<()> {
    save(kind.account(), value)
}

pub fn clear_ai_key(kind: AiKeyKind) -> Result<()> {
    clear(kind.account())
}
