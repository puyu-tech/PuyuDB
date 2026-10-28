use std::path::Path;

use adr02_protocol::{Engine, RootRow, Snapshot, pack_root, stream_seq_key, u64_key, unpack_root};
use heed::types::Bytes;
use heed::{Database, Env, EnvOpenOptions, PutFlags};

const MAP_SIZE: usize = 1 << 30;

struct Store {
    env: Box<Env>,
    chunks: Database<'static, Bytes, Bytes>,
    roots: Database<'static, Bytes, Bytes>,
    stream_seq: Database<'static, Bytes, Bytes>,
    events: Database<'static, Bytes, Bytes>,
    outbox: Database<'static, Bytes, Bytes>,
}

impl Store {
    fn open_path(path: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
        // SAFETY: heed documents that open is unsafe because LMDB is a memory map.
        // This process is the only opener of `path` unless probe_native says otherwise,
        // and the Env stays in a Box so its address does not change when Store moves.
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(MAP_SIZE)
                .max_dbs(8)
                .open(path)
                .map_err(|e| e.to_string())?
        };
        let env = Box::new(env);
        let wtxn = env.write_txn().map_err(|e| e.to_string())?;
        let created = env
            .create_databases_and_commit(wtxn, |opener, txn| {
                let chunks = opener.create_database::<Bytes, Bytes>(txn, Some("chunks"))?;
                let roots = opener.create_database::<Bytes, Bytes>(txn, Some("roots"))?;
                let stream_seq = opener.create_database::<Bytes, Bytes>(txn, Some("stream_seq"))?;
                let events = opener.create_database::<Bytes, Bytes>(txn, Some("events"))?;
                let outbox = opener.create_database::<Bytes, Bytes>(txn, Some("outbox"))?;
                Ok::<_, heed::Error>(heed::CommitOrAbort::<_, ()>::Commit((
                    chunks, roots, stream_seq, events, outbox,
                )))
            })
            .map_err(|e| e.to_string())?
            .unwrap_commit();
        // SAFETY: Database is Copy and stores only the LMDB dbi integer plus the Env address.
        // Env is boxed for the Store lifetime, so that address stays valid. Database has no Drop.
        let (chunks, roots, stream_seq, events, outbox) = unsafe {
            std::mem::transmute::<
                (
                    Database<'_, Bytes, Bytes>,
                    Database<'_, Bytes, Bytes>,
                    Database<'_, Bytes, Bytes>,
                    Database<'_, Bytes, Bytes>,
                    Database<'_, Bytes, Bytes>,
                ),
                (
                    Database<'static, Bytes, Bytes>,
                    Database<'static, Bytes, Bytes>,
                    Database<'static, Bytes, Bytes>,
                    Database<'static, Bytes, Bytes>,
                    Database<'static, Bytes, Bytes>,
                ),
            >(created)
        };
        Ok(Self {
            env,
            chunks,
            roots,
            stream_seq,
            events,
            outbox,
        })
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Self::open_path(&dir.join("heed"))
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        let mut txn = self.env.write_txn().map_err(|e| e.to_string())?;
        if self
            .chunks
            .get(&txn, key.as_slice())
            .map_err(|e| e.to_string())?
            .is_none()
        {
            self.chunks
                .put(&mut txn, key.as_slice(), bytes)
                .map_err(|e| e.to_string())?;
        }
        txn.commit().map_err(|e| e.to_string())
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        let mut txn = self.env.write_txn().map_err(|e| e.to_string())?;
        if self
            .chunks
            .get(&txn, key.as_slice())
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Err("chunk is not staged".into());
        }
        let ss = stream_seq_key(stream, seq);
        let root_key = u64_key(root);
        let packed = pack_root(key, stream, seq);
        self.stream_seq
            .put_with_flags(
                &mut txn,
                PutFlags::NO_OVERWRITE,
                ss.as_slice(),
                root_key.as_slice(),
            )
            .map_err(|e| format!("duplicate stream/seq or {e}"))?;
        self.roots
            .put_with_flags(
                &mut txn,
                PutFlags::NO_OVERWRITE,
                root_key.as_slice(),
                packed.as_slice(),
            )
            .map_err(|e| format!("duplicate root or {e}"))?;
        self.events
            .put_with_flags(
                &mut txn,
                PutFlags::NO_OVERWRITE,
                root_key.as_slice(),
                root_key.as_slice(),
            )
            .map_err(|e| e.to_string())?;
        let peer = 1u64.to_le_bytes();
        self.outbox
            .put_with_flags(
                &mut txn,
                PutFlags::NO_OVERWRITE,
                root_key.as_slice(),
                peer.as_slice(),
            )
            .map_err(|e| e.to_string())?;
        txn.commit().map_err(|e| e.to_string())
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let txn = self.env.read_txn().map_err(|e| e.to_string())?;
        let mut snap = Snapshot::default();
        for item in self.chunks.iter(&txn).map_err(|e| e.to_string())? {
            let (key, _) = item.map_err(|e| e.to_string())?;
            snap.chunks.push(copy32(key)?);
        }
        for item in self.roots.iter(&txn).map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            let root_id = u64::from_le_bytes(copy8(key)?);
            let (chunk_key, stream, seq) = unpack_root(value).ok_or("bad root value")?;
            snap.roots.push(RootRow {
                root_id,
                stream,
                seq,
                chunk_key,
            });
        }
        for item in self.events.iter(&txn).map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            snap.events.push((
                u64::from_le_bytes(copy8(key)?),
                u64::from_le_bytes(copy8(value)?),
            ));
        }
        for item in self.outbox.iter(&txn).map_err(|e| e.to_string())? {
            let (key, value) = item.map_err(|e| e.to_string())?;
            snap.outbox.push((
                u64::from_le_bytes(copy8(key)?),
                u64::from_le_bytes(copy8(value)?),
            ));
        }
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("heed");
        let first = Self::open_path(&path)?;
        let _held = first.env.write_txn().map_err(|e| e.to_string())?;
        match Self::open_path(&path) {
            Err(err) => Ok(format!("second_open=rejected:{err}")),
            Ok(second) => match second.env.write_txn() {
                Ok(_) => Ok("second_write=accepted".into()),
                Err(err) => Ok(format!("second_write=rejected:{err}")),
            },
        }
    }

    fn identify() -> String {
        "heed 0.23.1 lmdb header 0.9.70 default commit sync".into()
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
