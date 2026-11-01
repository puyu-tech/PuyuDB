use std::path::Path;

use adr02_protocol::{Engine, RootRow, Snapshot, pack_root, stream_seq_key, u64_key, unpack_root};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};

const CHUNKS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("chunks");
const ROOTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("roots");
const STREAM_SEQ: TableDefinition<&[u8], &[u8]> = TableDefinition::new("stream_seq");
const EVENTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("events");
const OUTBOX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("outbox");

struct Store {
    db: Database,
}

impl Store {
    fn open_path(path: &Path) -> Result<Database, String> {
        let db = if path.exists() {
            Database::open(path).map_err(|e| e.to_string())?
        } else {
            Database::create(path).map_err(|e| e.to_string())?
        };
        if !tables_ready(&db) {
            let mut txn = db.begin_write().map_err(|e| e.to_string())?;
            txn.set_durability(Durability::Immediate)
                .map_err(|e| e.to_string())?;
            let _ = txn.open_table(CHUNKS).map_err(|e| e.to_string())?;
            let _ = txn.open_table(ROOTS).map_err(|e| e.to_string())?;
            let _ = txn.open_table(STREAM_SEQ).map_err(|e| e.to_string())?;
            let _ = txn.open_table(EVENTS).map_err(|e| e.to_string())?;
            let _ = txn.open_table(OUTBOX).map_err(|e| e.to_string())?;
            txn.commit().map_err(|e| e.to_string())?;
        }
        Ok(db)
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Ok(Self {
            db: Self::open_path(&dir.join("store.redb"))?,
        })
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        let mut txn = self.db.begin_write().map_err(|e| e.to_string())?;
        txn.set_durability(Durability::Immediate)
            .map_err(|e| e.to_string())?;
        {
            let mut table = txn.open_table(CHUNKS).map_err(|e| e.to_string())?;
            if table
                .get(key.as_slice())
                .map_err(|e| e.to_string())?
                .is_none()
            {
                table
                    .insert(key.as_slice(), bytes)
                    .map_err(|e| e.to_string())?;
            }
        }
        txn.commit().map_err(|e| e.to_string())
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        let mut txn = self.db.begin_write().map_err(|e| e.to_string())?;
        txn.set_durability(Durability::Immediate)
            .map_err(|e| e.to_string())?;
        {
            let chunks = txn.open_table(CHUNKS).map_err(|e| e.to_string())?;
            if chunks
                .get(key.as_slice())
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err("chunk is not staged".into());
            }
            drop(chunks);
            let ss = stream_seq_key(stream, seq);
            let mut stream_seq = txn.open_table(STREAM_SEQ).map_err(|e| e.to_string())?;
            if stream_seq
                .get(ss.as_slice())
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("duplicate stream/seq".into());
            }
            let root_key = u64_key(root);
            let mut roots = txn.open_table(ROOTS).map_err(|e| e.to_string())?;
            if roots
                .get(root_key.as_slice())
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("duplicate root".into());
            }
            let packed = pack_root(key, stream, seq);
            roots
                .insert(root_key.as_slice(), packed.as_slice())
                .map_err(|e| e.to_string())?;
            stream_seq
                .insert(ss.as_slice(), root_key.as_slice())
                .map_err(|e| e.to_string())?;
            drop(roots);
            drop(stream_seq);
            let mut events = txn.open_table(EVENTS).map_err(|e| e.to_string())?;
            events
                .insert(root_key.as_slice(), root_key.as_slice())
                .map_err(|e| e.to_string())?;
            drop(events);
            let mut outbox = txn.open_table(OUTBOX).map_err(|e| e.to_string())?;
            let peer = 1u64.to_le_bytes();
            outbox
                .insert(root_key.as_slice(), peer.as_slice())
                .map_err(|e| e.to_string())?;
        }
        txn.commit().map_err(|e| e.to_string())
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let mut snap = Snapshot::default();
        let chunks = txn.open_table(CHUNKS).map_err(|e| e.to_string())?;
        for item in chunks.iter().map_err(|e| e.to_string())? {
            let (key, _) = item.map_err(|e| e.to_string())?;
            snap.chunks.push(copy32(key.value())?);
        }
        let roots = txn.open_table(ROOTS).map_err(|e| e.to_string())?;
        for item in roots.iter().map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            let root_id = u64::from_le_bytes(copy8(key.value())?);
            let (chunk_key, stream, seq) = unpack_root(value.value()).ok_or("bad root value")?;
            snap.roots.push(RootRow {
                root_id,
                stream,
                seq,
                chunk_key,
            });
        }
        let events = txn.open_table(EVENTS).map_err(|e| e.to_string())?;
        for item in events.iter().map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            snap.events.push((
                u64::from_le_bytes(copy8(key.value())?),
                u64::from_le_bytes(copy8(value.value())?),
            ));
        }
        let outbox = txn.open_table(OUTBOX).map_err(|e| e.to_string())?;
        for item in outbox.iter().map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            snap.outbox.push((
                u64::from_le_bytes(copy8(key.value())?),
                u64::from_le_bytes(copy8(value.value())?),
            ));
        }
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("store.redb");
        let first = Self::open_path(&path)?;
        let _held = first.begin_write().map_err(|e| e.to_string())?;
        let second = Self::open_path(&path);
        match second {
            Err(err) => Ok(format!("second_open=rejected:{err}")),
            Ok(db) => match db.begin_write() {
                Ok(_) => Ok("second_write=accepted".into()),
                Err(err) => Ok(format!("second_write=rejected:{err}")),
            },
        }
    }

    fn identify() -> String {
        "redb 4.4.0 durability=Immediate".into()
    }
}

fn tables_ready(db: &Database) -> bool {
    let Ok(txn) = db.begin_read() else {
        return false;
    };
    txn.open_table(CHUNKS).is_ok()
        && txn.open_table(ROOTS).is_ok()
        && txn.open_table(STREAM_SEQ).is_ok()
        && txn.open_table(EVENTS).is_ok()
        && txn.open_table(OUTBOX).is_ok()
}

fn copy32(bytes: &[u8]) -> Result<[u8; 32], String> {
    bytes
        .try_into()
        .map_err(|_| "expected 32 bytes".to_string())
}

fn copy8(bytes: &[u8]) -> Result<[u8; 8], String> {
    bytes.try_into().map_err(|_| "expected 8 bytes".to_string())
}

fn main() {
    if let Err(err) = adr02_protocol::main::<Store>() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
