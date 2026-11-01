use std::path::Path;

use adr02_protocol::{Engine, RootRow, Snapshot, pack_root, stream_seq_key, u64_key, unpack_root};
use fjall::{KeyspaceCreateOptions, PersistMode, SingleWriterTxDatabase, SingleWriterTxKeyspace};

struct Store {
    db: SingleWriterTxDatabase,
    chunks: SingleWriterTxKeyspace,
    roots: SingleWriterTxKeyspace,
    stream_seq: SingleWriterTxKeyspace,
    events: SingleWriterTxKeyspace,
    outbox: SingleWriterTxKeyspace,
}

impl Store {
    fn open_path(path: &Path) -> Result<Self, String> {
        let db = SingleWriterTxDatabase::builder(path)
            .open()
            .map_err(|e| e.to_string())?;
        let open = |name: &str| {
            db.keyspace(name, KeyspaceCreateOptions::default)
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            chunks: open("chunks")?,
            roots: open("roots")?,
            stream_seq: open("stream_seq")?,
            events: open("events")?,
            outbox: open("outbox")?,
            db,
        })
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Self::open_path(&dir.join("fjall"))
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        if self.chunks.get(key).map_err(|e| e.to_string())?.is_some() {
            return Ok(());
        }
        let mut tx = self.db.write_tx().durability(Some(PersistMode::SyncAll));
        tx.insert(&self.chunks, key.as_slice(), bytes);
        tx.commit().map_err(|e| e.to_string())
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        if self.chunks.get(key).map_err(|e| e.to_string())?.is_none() {
            return Err("chunk is not staged".into());
        }
        let ss = stream_seq_key(stream, seq);
        if self
            .stream_seq
            .get(ss)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("duplicate stream/seq".into());
        }
        let root_key = u64_key(root);
        if self
            .roots
            .get(root_key)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("duplicate root".into());
        }
        let packed = pack_root(key, stream, seq);
        let mut tx = self.db.write_tx().durability(Some(PersistMode::SyncAll));
        tx.insert(&self.roots, root_key.as_slice(), packed.as_slice());
        tx.insert(&self.stream_seq, ss.as_slice(), root_key.as_slice());
        tx.insert(&self.events, root_key.as_slice(), root_key.as_slice());
        tx.insert(
            &self.outbox,
            root_key.as_slice(),
            1u64.to_le_bytes().as_slice(),
        );
        tx.commit().map_err(|e| e.to_string())
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let mut snap = Snapshot::default();
        for guard in self.chunks.inner().iter() {
            let (key, _) = guard.into_inner().map_err(|e| e.to_string())?;
            snap.chunks.push(copy32(&key)?);
        }
        for guard in self.roots.inner().iter() {
            let (key, value) = guard.into_inner().map_err(|e| e.to_string())?;
            let root_id = u64::from_le_bytes(copy8(&key)?);
            let (chunk_key, stream, seq) = unpack_root(&value).ok_or("bad root value")?;
            snap.roots.push(RootRow {
                root_id,
                stream,
                seq,
                chunk_key,
            });
        }
        for guard in self.events.inner().iter() {
            let (key, value) = guard.into_inner().map_err(|e| e.to_string())?;
            snap.events.push((
                u64::from_le_bytes(copy8(&key)?),
                u64::from_le_bytes(copy8(&value)?),
            ));
        }
        for guard in self.outbox.inner().iter() {
            let (key, value) = guard.into_inner().map_err(|e| e.to_string())?;
            snap.outbox.push((
                u64::from_le_bytes(copy8(&key)?),
                u64::from_le_bytes(copy8(&value)?),
            ));
        }
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("fjall");
        let first = Self::open_path(&path)?;
        let mut held = first.db.write_tx();
        held.insert(&first.chunks, b"probe".as_slice(), b"1".as_slice());
        match Self::open_path(&path) {
            Err(err) => Ok(format!("second_open=rejected:{err}")),
            Ok(second) => {
                let mut tx = second.db.write_tx();
                tx.insert(&second.chunks, b"probe2".as_slice(), b"2".as_slice());
                match tx.commit() {
                    Ok(()) => Ok("second_write=accepted".into()),
                    Err(err) => Ok(format!("second_write=rejected:{err}")),
                }
            }
        }
    }

    fn identify() -> String {
        "fjall 3.1.13 single-writer tx persist=SyncAll".into()
    }
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
