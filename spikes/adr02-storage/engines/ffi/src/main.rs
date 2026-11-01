use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::ptr;

use adr02_protocol::{Engine, RootRow, SCHEMA, Snapshot};
use libsqlite3_sys as ffi;

struct Store {
    raw: *mut ffi::sqlite3,
}

impl Drop for Store {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: raw is an open sqlite3 handle owned by this Store, or null.
            unsafe { ffi::sqlite3_close(self.raw) };
        }
    }
}

impl Store {
    fn open_path(path: &Path) -> Result<Self, String> {
        let text = path.to_str().ok_or("path is not utf-8")?;
        let c_path = CString::new(text).map_err(|e| e.to_string())?;
        let mut raw = ptr::null_mut();
        // SAFETY: c_path is a valid C string. FULLMUTEX matches rusqlite's default serialized mode.
        let rc = unsafe {
            ffi::sqlite3_open_v2(
                c_path.as_ptr(),
                &mut raw,
                ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_FULLMUTEX,
                ptr::null(),
            )
        };
        let store = Self { raw };
        store.check(rc)?;
        store.exec("PRAGMA journal_mode=WAL")?;
        store.exec("PRAGMA synchronous=FULL")?;
        if std::env::var_os("ADR02_FULLFSYNC").is_some() {
            store.exec("PRAGMA fullfsync=ON")?;
        }
        store.exec("PRAGMA foreign_keys=ON")?;
        store.exec("PRAGMA busy_timeout=5000")?;
        store.exec(SCHEMA)?;
        Ok(store)
    }

    fn exec(&self, sql: &str) -> Result<(), String> {
        let c_sql = CString::new(sql).map_err(|e| e.to_string())?;
        let mut err = ptr::null_mut();
        // SAFETY: raw is open and c_sql is valid for this call. err is freed below if set.
        let rc =
            unsafe { ffi::sqlite3_exec(self.raw, c_sql.as_ptr(), None, ptr::null_mut(), &mut err) };
        if rc != ffi::SQLITE_OK {
            let message = unsafe { errmsg(self.raw, err) };
            if !err.is_null() {
                unsafe { ffi::sqlite3_free(err.cast()) };
            }
            return Err(format!("sqlite {rc}: {message}"));
        }
        Ok(())
    }

    fn check(&self, rc: c_int) -> Result<(), String> {
        if rc == ffi::SQLITE_OK {
            Ok(())
        } else {
            Err(format!("sqlite {rc}: {}", unsafe {
                errmsg(self.raw, ptr::null_mut())
            }))
        }
    }

    fn insert(
        &self,
        sql: &str,
        bind: impl FnOnce(*mut ffi::sqlite3_stmt) -> Result<(), String>,
    ) -> Result<(), String> {
        let c_sql = CString::new(sql).map_err(|e| e.to_string())?;
        let mut stmt = ptr::null_mut();
        // SAFETY: raw is open and c_sql is a valid SQL string. stmt is finalized before return.
        let rc = unsafe {
            ffi::sqlite3_prepare_v2(self.raw, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut())
        };
        self.check(rc)?;
        let result = (|| {
            bind(stmt)?;
            let rc = unsafe { ffi::sqlite3_step(stmt) };
            if rc != ffi::SQLITE_DONE {
                return Err(format!("sqlite {rc}: {}", unsafe {
                    errmsg(self.raw, ptr::null_mut())
                }));
            }
            Ok(())
        })();
        unsafe { ffi::sqlite3_finalize(stmt) };
        result
    }

    fn query(
        &self,
        sql: &str,
        mut row: impl FnMut(*mut ffi::sqlite3_stmt) -> Result<(), String>,
    ) -> Result<(), String> {
        let c_sql = CString::new(sql).map_err(|e| e.to_string())?;
        let mut stmt = ptr::null_mut();
        let rc = unsafe {
            ffi::sqlite3_prepare_v2(self.raw, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut())
        };
        self.check(rc)?;
        let result = (|| {
            loop {
                let rc = unsafe { ffi::sqlite3_step(stmt) };
                if rc == ffi::SQLITE_DONE {
                    return Ok(());
                }
                if rc != ffi::SQLITE_ROW {
                    return Err(format!("sqlite {rc}: {}", unsafe {
                        errmsg(self.raw, ptr::null_mut())
                    }));
                }
                row(stmt)?;
            }
        })();
        unsafe { ffi::sqlite3_finalize(stmt) };
        result
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Self::open_path(&dir.join("sqlite.db"))
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        self.insert(
            "INSERT OR IGNORE INTO chunks(key, bytes) VALUES (?1, ?2)",
            |stmt| {
                bind_blob(stmt, 1, key)?;
                bind_blob(stmt, 2, bytes)?;
                Ok(())
            },
        )
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        self.exec("BEGIN IMMEDIATE")?;
        let result = (|| {
            self.insert(
                "INSERT INTO roots(root_id, stream, seq, chunk_key) VALUES (?1, ?2, ?3, ?4)",
                |stmt| {
                    bind_i64(stmt, 1, root as i64)?;
                    bind_i64(stmt, 2, stream as i64)?;
                    bind_i64(stmt, 3, seq as i64)?;
                    bind_blob(stmt, 4, key)?;
                    Ok(())
                },
            )?;
            self.insert(
                "INSERT INTO events(cursor, root_id) VALUES (?1, ?1)",
                |stmt| bind_i64(stmt, 1, root as i64),
            )?;
            self.insert("INSERT INTO outbox(root_id, peer) VALUES (?1, 1)", |stmt| {
                bind_i64(stmt, 1, root as i64)
            })?;
            self.exec("COMMIT")
        })();
        if result.is_err() {
            let _ = self.exec("ROLLBACK");
        }
        result
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let mut snap = Snapshot::default();
        self.query("SELECT key FROM chunks", |stmt| {
            snap.chunks.push(column_key(stmt, 0)?);
            Ok(())
        })?;
        self.query(
            "SELECT root_id, stream, seq, chunk_key FROM roots",
            |stmt| {
                snap.roots.push(RootRow {
                    root_id: column_i64(stmt, 0) as u64,
                    stream: column_i64(stmt, 1) as u64,
                    seq: column_i64(stmt, 2) as u64,
                    chunk_key: column_key(stmt, 3)?,
                });
                Ok(())
            },
        )?;
        self.query("SELECT cursor, root_id FROM events", |stmt| {
            snap.events
                .push((column_i64(stmt, 0) as u64, column_i64(stmt, 1) as u64));
            Ok(())
        })?;
        self.query("SELECT root_id, peer FROM outbox", |stmt| {
            snap.outbox
                .push((column_i64(stmt, 0) as u64, column_i64(stmt, 1) as u64));
            Ok(())
        })?;
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("sqlite.db");
        let first = Self::open_path(&path)?;
        first.exec("BEGIN IMMEDIATE")?;
        let second = Self::open_path(&path)?;
        second.exec("PRAGMA busy_timeout=0")?;
        let write = second.exec("BEGIN IMMEDIATE");
        let _ = first.exec("ROLLBACK");
        match write {
            Ok(()) => Ok("second_begin=accepted".into()),
            Err(err) => Ok(format!("second_begin=rejected:{err}")),
        }
    }

    fn identify() -> String {
        let version = unsafe { CStr::from_ptr(ffi::sqlite3_libversion()) };
        format!(
            "libsqlite3-sys 0.39.1 bundled sqlite {}{}",
            version.to_string_lossy(),
            if std::env::var_os("ADR02_FULLFSYNC").is_some() {
                " fullfsync=on"
            } else {
                " fullfsync=default-off"
            }
        )
    }
}

fn bind_blob(stmt: *mut ffi::sqlite3_stmt, index: c_int, bytes: &[u8]) -> Result<(), String> {
    // SAFETY: stmt is an active statement. SQLITE_TRANSIENT copies the bytes before return.
    let rc = unsafe {
        ffi::sqlite3_bind_blob(
            stmt,
            index,
            bytes.as_ptr().cast(),
            bytes.len() as c_int,
            ffi::SQLITE_TRANSIENT(),
        )
    };
    if rc == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(format!("bind blob {rc}"))
    }
}

fn bind_i64(stmt: *mut ffi::sqlite3_stmt, index: c_int, value: i64) -> Result<(), String> {
    let rc = unsafe { ffi::sqlite3_bind_int64(stmt, index, value) };
    if rc == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(format!("bind int {rc}"))
    }
}

fn column_i64(stmt: *mut ffi::sqlite3_stmt, index: c_int) -> i64 {
    unsafe { ffi::sqlite3_column_int64(stmt, index) }
}

fn column_key(stmt: *mut ffi::sqlite3_stmt, index: c_int) -> Result<[u8; 32], String> {
    let ptr = unsafe { ffi::sqlite3_column_blob(stmt, index) };
    let len = unsafe { ffi::sqlite3_column_bytes(stmt, index) };
    if ptr.is_null() || len != 32 {
        return Err(format!("blob column {index} is not 32 bytes"));
    }
    let mut key = [0u8; 32];
    // SAFETY: sqlite3_column_blob is valid until the next column call on this statement, and len is 32.
    key.copy_from_slice(unsafe { std::slice::from_raw_parts(ptr.cast(), 32) });
    Ok(key)
}

/// # Safety
/// `raw` is an open database or null. `err` is a sqlite-owned message or null.
unsafe fn errmsg(raw: *mut ffi::sqlite3, err: *mut c_char) -> String {
    let ptr = if err.is_null() {
        unsafe { ffi::sqlite3_errmsg(raw) }
    } else {
        err
    };
    if ptr.is_null() {
        return "unknown sqlite error".into();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn main() {
    if let Err(err) = adr02_protocol::main::<Store>() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
