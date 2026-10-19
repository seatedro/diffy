use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use rusqlite::{Connection, Row};
use sea_query::{
    ColumnDef, Expr, ExprTrait, Iden, OnConflict, Query, SelectStatement, SqliteQueryBuilder,
    Table, TableCreateStatement,
};
use sea_query_rusqlite::{RusqliteBinder, RusqliteValues};

use crate::core::error::{DiffyError, Result};

pub const DATABASE_FILE_NAME: &str = "diffy.db";
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Iden)]
pub enum Meta {
    Table,
    Key,
    Value,
}

#[derive(Iden)]
pub enum Settings {
    Table,
    Id,
    Json,
}

#[derive(Iden)]
pub enum Frecency {
    Table,
    Key,
    AccessCount,
    LastAccessMs,
}

#[derive(Iden)]
pub enum ReviewSessions {
    Table,
    Key,
    Json,
}

#[derive(Iden)]
pub enum Repositories {
    Table,
    Path,
    Branch,
    Active,
    Discovered,
}

fn schema_v1() -> Vec<TableCreateStatement> {
    vec![
        Table::create()
            .table(Meta::Table)
            .col(ColumnDef::new(Meta::Key).text().primary_key())
            .col(ColumnDef::new(Meta::Value).text().not_null())
            .to_owned(),
        Table::create()
            .table(Settings::Table)
            .col(ColumnDef::new(Settings::Id).integer().primary_key())
            .col(ColumnDef::new(Settings::Json).text().not_null())
            .to_owned(),
        Table::create()
            .table(Frecency::Table)
            .col(ColumnDef::new(Frecency::Key).text().primary_key())
            .col(ColumnDef::new(Frecency::AccessCount).integer().not_null())
            .col(
                ColumnDef::new(Frecency::LastAccessMs)
                    .big_integer()
                    .not_null(),
            )
            .to_owned(),
        Table::create()
            .table(ReviewSessions::Table)
            .col(ColumnDef::new(ReviewSessions::Key).text().primary_key())
            .col(ColumnDef::new(ReviewSessions::Json).text().not_null())
            .to_owned(),
        Table::create()
            .table(Repositories::Table)
            .col(ColumnDef::new(Repositories::Path).text().primary_key())
            .col(ColumnDef::new(Repositories::Branch).text().null())
            .col(ColumnDef::new(Repositories::Active).big_integer().null())
            .col(
                ColumnDef::new(Repositories::Discovered)
                    .boolean()
                    .not_null(),
            )
            .to_owned(),
    ]
}

const MIGRATIONS: &[fn() -> Vec<TableCreateStatement>] = &[schema_v1];

#[derive(Clone)]
pub struct Database {
    path: Arc<PathBuf>,
    connection: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Database {
    pub fn in_dir(dir: &Path) -> Result<Self> {
        static OPEN: OnceLock<Mutex<HashMap<PathBuf, Database>>> = OnceLock::new();
        let mut open = OPEN
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| DiffyError::General("database registry poisoned".to_owned()))?;
        if let Some(database) = open.get(dir) {
            return Ok(database.clone());
        }
        let database = Self::open(&dir.join(DATABASE_FILE_NAME))?;
        open.insert(dir.to_path_buf(), database.clone());
        Ok(database)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&mut connection)?;
        Ok(Self {
            path: Arc::new(path.to_path_buf()),
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| DiffyError::General("database connection poisoned".to_owned()))
    }

    pub fn execute(&self, statement: &impl RusqliteBinder) -> Result<usize> {
        let (sql, values) = statement.build_rusqlite(SqliteQueryBuilder);
        Ok(self.connection()?.execute(&sql, &*values.as_params())?)
    }

    pub fn query<T>(
        &self,
        statement: &SelectStatement,
        mut map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let (sql, values) = statement.build_rusqlite(SqliteQueryBuilder);
        let connection = self.connection()?;
        let mut prepared = connection.prepare(&sql)?;
        let rows = prepared.query_map(&*values.as_params(), |row| map(row))?;
        Ok(rows.collect::<rusqlite::Result<Vec<T>>>()?)
    }

    pub fn transaction(&self, statements: Vec<(String, RusqliteValues)>) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for (sql, values) in &statements {
            transaction.execute(sql, &*values.as_params())?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn import_once(&self, name: &str, import: impl FnOnce(&Database) -> Result<()>) {
        let key = format!("imported:{name}");
        let done = Query::select()
            .column(Meta::Value)
            .from(Meta::Table)
            .and_where(Expr::col(Meta::Key).eq(key.as_str()))
            .to_owned();
        let result = self
            .query(&done, |row| row.get::<_, String>(0))
            .and_then(|rows| {
                if !rows.is_empty() {
                    return Ok(());
                }
                import(self)?;
                self.execute(
                    &Query::insert()
                        .into_table(Meta::Table)
                        .columns([Meta::Key, Meta::Value])
                        .values_panic([key.as_str().into(), "1".into()])
                        .on_conflict(OnConflict::column(Meta::Key).do_nothing().to_owned())
                        .to_owned(),
                )?;
                Ok(())
            });
        if let Err(error) = result {
            tracing::warn!(store = name, %error, "importing the previous store failed");
        }
    }
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let version: usize = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(version) {
        let transaction = connection.transaction()?;
        for statement in migration() {
            transaction.execute_batch(&statement.to_string(SqliteQueryBuilder))?;
        }
        transaction.pragma_update(None, "user_version", index + 1)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrates_once_and_shares_a_connection_per_dir() {
        let dir = TempDir::new().unwrap();
        let first = Database::in_dir(dir.path()).unwrap();
        let second = Database::in_dir(dir.path()).unwrap();
        assert!(Arc::ptr_eq(&first.connection, &second.connection));
        let version: usize = first
            .connection()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len());

        let mut runs = 0;
        first.import_once("probe", |_| {
            runs += 1;
            Ok(())
        });
        second.import_once("probe", |_| {
            runs += 1;
            Ok(())
        });
        assert_eq!(runs, 1);
    }
}
