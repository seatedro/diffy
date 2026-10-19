use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sea_query::{Expr, ExprTrait, OnConflict, Query};
use serde::{Deserialize, Serialize};

use crate::core::error::Result;
use crate::core::review::{ReviewSession, ReviewSessionKey, ReviewTarget};
use crate::platform::database::{Database, ReviewSessions};
use crate::platform::persistence::SettingsStore;

const REVIEW_STORE_FILE_NAME: &str = "review-sessions.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
struct ReviewStoreData {
    sessions: BTreeMap<String, ReviewSession>,
}

#[derive(Debug, Clone)]
pub struct ReviewStore {
    dir: PathBuf,
}

impl ReviewStore {
    pub fn new_default() -> Self {
        let base = crate::platform::startup::app_config_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self { dir: base }
    }

    pub fn for_settings_store(settings_store: &SettingsStore) -> Self {
        Self {
            dir: settings_store.dir().to_path_buf(),
        }
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let mut dir = path.into();
        if dir.extension().is_some() {
            dir.pop();
        }
        let store = Self { dir };
        store.database()?;
        Ok(store)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load_session(
        &self,
        target: &ReviewTarget,
        head_sha: &str,
    ) -> Result<Option<ReviewSession>> {
        let key = ReviewSessionKey::new(target.clone(), head_sha.to_owned()).storage_key();
        let select = Query::select()
            .column(ReviewSessions::Json)
            .from(ReviewSessions::Table)
            .and_where(Expr::col(ReviewSessions::Key).eq(key))
            .to_owned();
        let rows = self
            .database()?
            .query(&select, |row| row.get::<_, String>(0))?;
        Ok(rows.first().and_then(|json| parse(json)))
    }

    pub fn load_sessions_for_target(&self, target: &ReviewTarget) -> Result<Vec<ReviewSession>> {
        let select = Query::select()
            .column(ReviewSessions::Json)
            .from(ReviewSessions::Table)
            .to_owned();
        let rows = self
            .database()?
            .query(&select, |row| row.get::<_, String>(0))?;
        Ok(rows
            .iter()
            .filter_map(|json| parse(json))
            .filter(|session| &session.target == target)
            .collect())
    }

    pub fn save_session(&self, session: &ReviewSession) -> Result<()> {
        write_session(
            &self.database()?,
            &session.key().storage_key(),
            &serde_json::to_string(session)?,
        )
    }

    pub fn remove_session(&self, key: &ReviewSessionKey) -> Result<()> {
        self.database()?.execute(
            &Query::delete()
                .from_table(ReviewSessions::Table)
                .and_where(Expr::col(ReviewSessions::Key).eq(key.storage_key()))
                .to_owned(),
        )?;
        Ok(())
    }

    fn database(&self) -> Result<Database> {
        let db = Database::in_dir(&self.dir)?;
        let legacy = self.dir.join(REVIEW_STORE_FILE_NAME);
        db.import_once("review_sessions", |db| {
            let Ok(contents) = fs::read(&legacy) else {
                return Ok(());
            };
            let data: ReviewStoreData = serde_json::from_slice(&contents).unwrap_or_else(|error| {
                tracing::warn!("corrupt review store at {}: {error}", legacy.display());
                ReviewStoreData::default()
            });
            for (key, session) in data.sessions {
                write_session(db, &key, &serde_json::to_string(&session)?)?;
            }
            Ok(())
        });
        Ok(db)
    }
}

fn parse(json: &str) -> Option<ReviewSession> {
    serde_json::from_str(json)
        .map_err(|error| tracing::warn!("skipping unreadable review session: {error}"))
        .ok()
}

fn write_session(db: &Database, key: &str, json: &str) -> Result<()> {
    db.execute(
        &Query::insert()
            .into_table(ReviewSessions::Table)
            .columns([ReviewSessions::Key, ReviewSessions::Json])
            .values_panic([key.into(), json.into()])
            .on_conflict(
                OnConflict::column(ReviewSessions::Key)
                    .update_column(ReviewSessions::Json)
                    .to_owned(),
            )
            .to_owned(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::ReviewStore;
    use crate::core::forge::github::PullRequestInfo;
    use crate::core::review::{ReviewSession, ReviewTarget};

    #[test]
    fn review_sessions_round_trip_by_head_sha() {
        let dir = TempDir::new().unwrap();
        let store = ReviewStore::open(dir.path()).unwrap();
        let target = ReviewTarget::github("owner", "repo", 7);
        let session = ReviewSession::new(
            target.clone(),
            PullRequestInfo {
                number: 7,
                head_sha: "abc123".to_owned(),
                ..PullRequestInfo::default()
            },
        );

        store.save_session(&session).unwrap();
        let reopened = ReviewStore::open(dir.path()).unwrap();
        assert_eq!(
            reopened.load_session(&target, "abc123").unwrap(),
            Some(session)
        );
        assert!(
            reopened
                .load_session(&target, "different")
                .unwrap()
                .is_none()
        );
    }
}
