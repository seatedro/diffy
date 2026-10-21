use std::path::Path;
use std::time::Duration;

use diffy_syntax::{Symbol, SymbolKind};
use rusqlite::{Connection, OptionalExtension, params};

use crate::core::error::Result;

const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_BYTES: i64 = 256 << 20;
const RECORD_BYTES: usize = 21;

pub type BlobKey = ([u8; 20], u64);

pub struct SymbolCache {
    connection: Connection,
    max_bytes: i64,
}

impl std::fmt::Debug for SymbolCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SymbolCache").finish_non_exhaustive()
    }
}

impl SymbolCache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA auto_vacuum = INCREMENTAL;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS symbols (
                 blob BLOB NOT NULL,
                 tagger INTEGER NOT NULL,
                 used INTEGER NOT NULL,
                 size INTEGER NOT NULL,
                 data BLOB NOT NULL,
                 PRIMARY KEY (blob, tagger)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS symbols_used ON symbols (used);",
        )?;
        Ok(Self {
            connection,
            max_bytes: MAX_BYTES,
        })
    }

    pub fn get_many(&self, keys: &[BlobKey], now: i64) -> Result<Vec<Option<Vec<Symbol>>>> {
        let mut select = self
            .connection
            .prepare_cached("SELECT used, data FROM symbols WHERE blob = ?1 AND tagger = ?2")?;
        let mut stale = Vec::new();
        let mut found = Vec::with_capacity(keys.len());
        for key in keys {
            let row: Option<(i64, Vec<u8>)> = select
                .query_row(params![&key.0[..], key.1 as i64], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()?;
            found.push(row.and_then(|(used, data)| {
                if now - used > TOUCH_AFTER.as_secs() as i64 {
                    stale.push(*key);
                }
                decode(&data)
            }));
        }
        drop(select);
        if !stale.is_empty() {
            let tx = self.connection.unchecked_transaction()?;
            {
                let mut touch = tx.prepare_cached(
                    "UPDATE symbols SET used = ?1 WHERE blob = ?2 AND tagger = ?3",
                )?;
                for key in stale {
                    touch.execute(params![now, &key.0[..], key.1 as i64])?;
                }
            }
            tx.commit()?;
        }
        Ok(found)
    }

    pub fn put_many(&mut self, entries: &[(BlobKey, &[Symbol])], now: i64) -> Result<()> {
        let tx = self.connection.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO symbols (blob, tagger, used, size, data)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (key, symbols) in entries {
                let data = encode(symbols);
                insert.execute(params![
                    &key.0[..],
                    key.1 as i64,
                    now,
                    data.len() as i64 + 40,
                    data
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn prune(&mut self, now: i64) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM symbols WHERE used < ?1",
            params![now - MAX_AGE.as_secs() as i64],
        )?;
        let total: i64 = tx.query_row("SELECT COALESCE(SUM(size), 0) FROM symbols", [], |row| {
            row.get(0)
        })?;
        if total > self.max_bytes {
            let mut excess = total - self.max_bytes * 3 / 4;
            let mut cutoff = None;
            {
                let mut oldest = tx.prepare("SELECT used, size FROM symbols ORDER BY used")?;
                let mut rows = oldest.query([])?;
                while let Some(row) = rows.next()? {
                    let used: i64 = row.get(0)?;
                    excess -= row.get::<_, i64>(1)?;
                    if excess <= 0 {
                        cutoff = Some(used);
                        break;
                    }
                }
            }
            match cutoff {
                Some(used) => tx.execute("DELETE FROM symbols WHERE used <= ?1", params![used])?,
                None => tx.execute("DELETE FROM symbols", [])?,
            };
        }
        tx.commit()?;
        self.connection
            .execute_batch("PRAGMA incremental_vacuum;")?;
        Ok(())
    }

    #[cfg(test)]
    fn len(&self) -> i64 {
        self.connection
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
            .unwrap_or_default()
    }
}

fn encode(symbols: &[Symbol]) -> Vec<u8> {
    let names: usize = symbols.iter().map(|symbol| symbol.name.len()).sum();
    let mut out = Vec::with_capacity(symbols.len() * RECORD_BYTES + names);
    for symbol in symbols {
        out.push(symbol.kind as u8);
        for value in [
            symbol.line,
            symbol.column,
            symbol.start_line,
            symbol.end_line,
            symbol.name.len() as u32,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(symbol.name.as_bytes());
    }
    out
}

fn decode(mut data: &[u8]) -> Option<Vec<Symbol>> {
    let mut symbols = Vec::new();
    while !data.is_empty() {
        let (head, rest) = data.split_at_checked(RECORD_BYTES)?;
        let field = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let (name, rest) = rest.split_at_checked(field(17) as usize)?;
        symbols.push(Symbol {
            name: Box::from(std::str::from_utf8(name).ok()?),
            kind: SymbolKind::from_u8(head[0]),
            line: field(1),
            column: field(5),
            start_line: field(9),
            end_line: field(13),
        });
        data = rest;
    }
    Some(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, line: u32) -> Symbol {
        Symbol {
            name: Box::from(name),
            kind: SymbolKind::Function,
            line,
            column: 3,
            start_line: line,
            end_line: line + 2,
        }
    }

    fn key(byte: u8) -> BlobKey {
        ([byte; 20], 7)
    }

    #[test]
    fn round_trips_and_misses_other_taggers() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = SymbolCache::open(&dir.path().join("symbols.db")).unwrap();
        let symbols = vec![symbol("alpha", 1), symbol("beta", 9)];
        cache.put_many(&[(key(1), &symbols)], 100).unwrap();
        let found = cache
            .get_many(&[key(1), ([1; 20], 8), key(2)], 100)
            .unwrap();
        assert_eq!(found, vec![Some(symbols), None, None]);
    }

    #[test]
    fn prune_drops_expired_then_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = SymbolCache::open(&dir.path().join("symbols.db")).unwrap();
        let symbols = vec![symbol("alpha", 1)];
        let day = TOUCH_AFTER.as_secs() as i64;
        let now = MAX_AGE.as_secs() as i64 + 10 * day;
        cache.put_many(&[(key(1), &symbols)], 0).unwrap();
        cache
            .put_many(&[(key(2), &symbols)], now - 3 * day)
            .unwrap();
        cache
            .put_many(&[(key(3), &symbols)], now - 2 * day)
            .unwrap();
        cache.put_many(&[(key(4), &symbols)], now - day).unwrap();
        cache.get_many(&[key(2)], now).unwrap();
        cache.max_bytes = 2 * (encode(&symbols).len() as i64 + 40);
        cache.prune(now).unwrap();
        let found = cache
            .get_many(&[key(1), key(2), key(3), key(4)], now)
            .unwrap();
        assert_eq!(found, vec![None, Some(symbols), None, None]);
        assert_eq!(cache.len(), 1);
    }
}
