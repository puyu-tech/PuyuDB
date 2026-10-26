// SPDX-License-Identifier: Apache-2.0
//! Deterministic bounded smoke/property screening, not coverage-guided fuzzing.
//!
//! Run exactly one trial per watchdog-protected process: `screen ENGINE INDEX`.
//! `screen ENGINE list` only constructs native valid fixtures and lists mutants.
//! Engines: dt-binding, dt-native, loro-binding, loro-native.
//! stdout (no header for a trial):
//! engine,layer,case_index,family,expectation,risk,checksum,encoded_bytes,count,
//! errors,accepts,panics,base_mutated,snapshot_panics,violations,elapsed_us
//!
//! stderr retains exact seed, valid update, and candidate hex BEFORE decoding.
//! Exit 0: completed observation/control pass; 1: diagnostic failure; 2: setup.
//! Raw native decoders may mutate on error; successful native mutation is normal.
//! Binding bases must stay unchanged on every outcome. Catching an unwind is only
//! diagnostic: aborts, allocator failure, UB and native crashes need OS isolation.

#[allow(dead_code)]
#[path = "compare.rs"]
mod compare;

use std::{
    fmt::Write as _,
    io::{self, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    process::ExitCode,
    time::Instant,
};

use compare::loro_fit;
use diamond_types::list::{ListOpLog, encoding::EncodeOptions};
use loro::{ExportMode, LoroDoc};

const MAX_ENCODED: usize = 64 * 1024;
const MAX_CASES: usize = 600;
const EPOCH: [u8; 32] = [7; 32];
const BASELINE: &str = "aé中🦀z";

#[derive(Clone, Copy, Debug)]
enum Engine {
    DtBinding,
    DtNative,
    LoroBinding,
    LoroNative,
}

impl Engine {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "dt-binding" => Ok(Self::DtBinding),
            "dt-native" => Ok(Self::DtNative),
            "loro-binding" => Ok(Self::LoroBinding),
            "loro-native" => Ok(Self::LoroNative),
            _ => Err(format!("unknown engine: {name}")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DtBinding => "dt-binding",
            Self::DtNative => "dt-native",
            Self::LoroBinding => "loro-binding",
            Self::LoroNative => "loro-native",
        }
    }

    fn layer(self) -> &'static str {
        match self {
            Self::DtBinding => "final_strict_binding",
            Self::DtNative => "native_decode_and_add",
            Self::LoroBinding => "experimental_inspect_replay_reencode",
            Self::LoroNative => "native_import",
        }
    }

    fn is_dt(self) -> bool {
        matches!(self, Self::DtBinding | Self::DtNative)
    }

    fn is_binding(self) -> bool {
        matches!(self, Self::DtBinding | Self::LoroBinding)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expectation {
    MustAccept,
    MustReject,
    Observe,
}

impl Expectation {
    fn name(self) -> &'static str {
        match self {
            Self::MustAccept => "must_accept",
            Self::MustReject => "must_reject",
            Self::Observe => "observe",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Case {
    family: String,
    bytes: Vec<u8>,
    binding: Expectation,
    native: Expectation,
    risk: &'static str,
    checksum: &'static str,
}

impl Case {
    fn observation(family: impl Into<String>, bytes: Vec<u8>, checksum: &'static str) -> Self {
        Self {
            family: family.into(),
            bytes,
            binding: Expectation::Observe,
            native: Expectation::Observe,
            risk: "normal",
            checksum,
        }
    }

    fn control(family: &str, bytes: Vec<u8>, binding: Expectation, native: Expectation) -> Self {
        Self {
            binding,
            native,
            ..Self::observation(family, bytes, "native")
        }
    }

    fn expectation(&self, engine: Engine) -> Expectation {
        if engine.is_binding() {
            self.binding
        } else {
            self.native
        }
    }
}

struct LoroFixture {
    base: loro_fit::FixtureBase,
    binding: loro_fit::Binding,
    live: LoroDoc,
}

enum Fixture {
    Dt(ListOpLog),
    Loro(LoroFixture),
}

impl Fixture {
    fn seed_bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Dt(base) => Ok(base.encode(&EncodeOptions::full().compress_content(false))),
            Self::Loro(fixture) => fixture.live.export(ExportMode::Snapshot).map_err(error),
        }
    }

    fn snapshot(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Dt(base) => {
                let mut bytes = base.encode(&EncodeOptions::full().compress_content(false));
                // Native export can omit unused declarations added before an
                // error; count them too rather than hiding that base mutation.
                bytes.extend(base.num_agents().to_le_bytes());
                Ok(bytes)
            }
            Self::Loro(fixture) => {
                // Observe both the live native receiver and the actual binding
                // base (via its public trusted-snapshot reconstruction API).
                let mut bytes = fixture.live.get_pending_txn_len().to_le_bytes().to_vec();
                bytes.extend(fixture.live.export(ExportMode::Snapshot).map_err(error)?);
                bytes.extend(
                    fixture
                        .base
                        .document()
                        .map_err(error)?
                        .export(ExportMode::Snapshot)
                        .map_err(error)?,
                );
                Ok(bytes)
            }
        }
    }

    fn attempt(&mut self, engine: Engine, bytes: &[u8]) -> Result<(), String> {
        match (self, engine) {
            (Self::Dt(base), Engine::DtBinding) => compare::dt_screen_inspect(base, bytes),
            (Self::Dt(base), Engine::DtNative) => {
                base.decode_and_add(bytes).map(|_| ()).map_err(error)
            }
            (Self::Loro(fixture), Engine::LoroBinding) => {
                // Never reuse the valid artifact's stale outer SHA as an oracle.
                let candidate = loro_fit::CapturedUpdate::new(bytes.to_vec());
                loro_fit::inspect(&fixture.base, &fixture.binding, 1, &candidate)
                    .map(|_| ())
                    .map_err(error)
            }
            (Self::Loro(fixture), Engine::LoroNative) => {
                fixture.live.import(bytes).map(|_| ()).map_err(error)
            }
            _ => Err("fixture/engine mismatch".into()),
        }
    }
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn fixtures(engine: Engine) -> Result<(Fixture, Vec<u8>, Vec<Case>), String> {
    use Expectation::{MustAccept, MustReject, Observe};
    if engine.is_dt() {
        let (base, valid) = compare::dt_screen_fixture();
        let mut controls = vec![Case::control(
            "valid_unicode_update",
            valid.clone(),
            MustAccept,
            MustAccept,
        )];
        controls.push(Case::control(
            "known_seed_replay",
            base.encode(&EncodeOptions::patch().compress_content(false)),
            MustReject,
            MustAccept,
        ));
        let (stage, _) = base.decode_new_strict(&valid).map_err(error)?;
        controls.push(Case::control(
            "wrong_binding_metadata",
            stage.encode_from(
                &EncodeOptions::patch()
                    .compress_content(false)
                    .user_data(b"wrong binding metadata"),
                base.local_frontier_ref(),
            ),
            MustReject,
            MustAccept,
        ));
        let mut wrong_magic = valid.clone();
        wrong_magic[0] ^= 1;
        controls.push(Case::control(
            "wrong_magic",
            wrong_magic,
            MustReject,
            MustReject,
        ));
        let mut wrong_crc = valid.clone();
        *wrong_crc.last_mut().ok_or("missing DT CRC")? ^= 1;
        controls.push(Case::control(
            "wrong_checksum",
            wrong_crc,
            MustReject,
            MustReject,
        ));
        controls.push(Case::control(
            "missing_required_crc",
            valid[..valid.len() - 6].to_vec(),
            MustReject,
            Observe,
        ));
        Ok((Fixture::Dt(base), valid, controls))
    } else {
        let base = loro_fit::FixtureBase::seed(EPOCH, BASELINE).map_err(error)?;
        let writer = loro_fit::FixtureWriter {
            public_key: [2; 32],
            branch: [3; 16],
            writer_epoch: [4; 16],
        };
        let binding = loro_fit::EpochPeers::new(EPOCH)
            .bind(writer, 2)
            .map_err(error)?;
        let valid = loro_fit::author(
            &base,
            &binding,
            1,
            &[loro_fit::ByteEdit {
                range: 1..3,
                inserted: "e\u{301}🌍",
            }],
        )
        .map_err(error)?
        .bytes;
        verify_loro_checksum(&valid)?;
        let live = base.document().map_err(error)?;
        let seed = live.export(ExportMode::Snapshot).map_err(error)?;
        verify_loro_checksum(&seed)?;
        let mut controls = vec![
            Case::control(
                "valid_unicode_update",
                valid.clone(),
                MustAccept,
                MustAccept,
            ),
            Case::control("snapshot_not_one_update", seed, MustReject, MustAccept),
            Case::control(
                "known_seed_replay",
                live.export(ExportMode::all_updates()).map_err(error)?,
                MustReject,
                MustAccept,
            ),
        ];
        let wrong_sequence = loro_fit::author(
            &base,
            &binding,
            2,
            &[loro_fit::ByteEdit {
                range: 0..0,
                inserted: "X",
            }],
        )
        .map_err(error)?;
        controls.push(Case::control(
            "wrong_binding_message",
            wrong_sequence.bytes,
            MustReject,
            MustAccept,
        ));
        let foreign = live.fork();
        foreign.set_peer_id(99).map_err(error)?;
        let from = foreign.oplog_vv();
        foreign.get_text("text").insert(0, "X").map_err(error)?;
        foreign.commit();
        controls.push(Case::control(
            "foreign_native_peer",
            foreign.export(ExportMode::updates(&from)).map_err(error)?,
            MustReject,
            MustAccept,
        ));
        let multiple = live.fork();
        multiple.set_peer_id(2).map_err(error)?;
        multiple.set_change_merge_interval(-1);
        let from = multiple.oplog_vv();
        for text in ["X", "Y"] {
            multiple.get_text("text").insert(0, text).map_err(error)?;
            multiple.commit();
        }
        controls.push(Case::control(
            "two_native_changes",
            multiple.export(ExportMode::updates(&from)).map_err(error)?,
            MustReject,
            MustAccept,
        ));
        let mut wrong_magic = valid.clone();
        wrong_magic[0] ^= 1;
        controls.push(Case::control(
            "wrong_magic",
            wrong_magic,
            MustReject,
            MustReject,
        ));
        let mut wrong_checksum = valid.clone();
        wrong_checksum[16] ^= 1;
        controls.push(Case::control(
            "wrong_checksum",
            wrong_checksum,
            MustReject,
            MustReject,
        ));
        // FastUpdates store literal text; a 32 KiB update stays under our cap.
        let large_update = loro_fit::author(
            &base,
            &binding,
            1,
            &[loro_fit::ByteEdit {
                range: 0..0,
                inserted: &"a".repeat(32 * 1024),
            }],
        )
        .map_err(error)?
        .bytes;
        verify_loro_checksum(&large_update)?;
        let mut large_case = Case::control(
            "native_default_update_32k",
            large_update,
            MustAccept,
            MustAccept,
        );
        large_case.risk = "high";
        controls.push(large_case);
        // Authentic default snapshots use native LZ4 KV blocks. Do not handcraft
        // compressed streams or author an over-cap literal FastUpdates blob.
        let expanded = live.fork();
        expanded.set_peer_id(2).map_err(error)?;
        expanded
            .get_text("text")
            .insert(0, &"a".repeat(128 * 1024))
            .map_err(error)?;
        expanded.commit();
        let compressed = expanded.export(ExportMode::Snapshot).map_err(error)?;
        verify_loro_checksum(&compressed)?;
        if compressed.len() > MAX_ENCODED {
            return Err("native compressed snapshot exceeds encoded cap".into());
        }
        let mut expansion_case = Case::control(
            "native_default_snapshot_expansion_128k",
            compressed,
            MustReject,
            MustAccept,
        );
        expansion_case.risk = "high";
        controls.push(expansion_case);
        Ok((
            Fixture::Loro(LoroFixture {
                base,
                binding,
                live,
            }),
            valid,
            controls,
        ))
    }
}

fn corpus(engine: Engine, valid: &[u8], mut cases: Vec<Case>) -> Result<Vec<Case>, String> {
    use Expectation::{MustReject, Observe};
    // Full offset coverage of the SMALL native update, not the expansion fixture.
    for offset in 0..valid.len() {
        let mut case = Case::observation(
            format!("truncate_{offset}"),
            valid[..offset].to_vec(),
            "unrepaired",
        );
        // Only header-short truncations have an unconditional cross-engine rule.
        // Other truncations are observations: replay/framing can ignore suffixes.
        if offset < if engine.is_dt() { 9 } else { 22 } {
            case.binding = MustReject;
            case.native = MustReject;
        }
        cases.push(case);
    }
    for offset in [0, valid.len() / 4, valid.len() / 2, valid.len() - 1] {
        for xor in [1, 128, 255] {
            let mut bytes = valid.to_vec();
            bytes[offset] ^= xor;
            mutation_pair(&mut cases, engine, format!("xor_{offset}_{xor}"), bytes);
        }
        for value in [0, 255] {
            let mut bytes = valid.to_vec();
            bytes[offset] = value;
            mutation_pair(&mut cases, engine, format!("byte_{offset}_{value}"), bytes);
        }
    }
    for (name, suffix) in [
        ("trailing_zero", vec![0]),
        ("trailing_junk", b"unconsumed".to_vec()),
        ("concatenated_update", valid.to_vec()),
    ] {
        let mut bytes = valid.to_vec();
        bytes.extend(suffix);
        let mut case = Case::observation(name, bytes, "unrepaired");
        // DT's strict final CRC must exhaust the complete submitted input.
        case.binding = if engine.is_dt() { MustReject } else { Observe };
        if !engine.is_dt() {
            // Current Loro checksums cover the suffix too; retain a repaired
            // companion so framing probes do not stop at the checksum checker.
            cases.push(repaired_case(
                engine,
                format!("{name}_sealed"),
                case.bytes.clone(),
            ));
        }
        cases.push(case);
    }
    if engine.is_dt() {
        let prefix = &valid[..valid.len() - 6];
        for tag in [99, 11, 27] {
            let mut bytes = prefix.to_vec();
            bytes.splice(9..9, [tag, 0]);
            let mut case = Case::observation(
                format!("unknown_or_unsupported_chunk_{tag}"),
                dt_seal(bytes),
                "crc32c_repaired",
            );
            case.binding = MustReject;
            cases.push(case);
        }
        // An unsupported compression marker is NOT compressed-payload screening.
        let mut bytes = prefix.to_vec();
        bytes.splice(9..9, [5, 0]);
        let mut case = Case::observation(
            "unsupported_compression_marker",
            dt_seal(bytes),
            "crc32c_repaired",
        );
        case.binding = MustReject;
        cases.push(case);
    } else {
        let mut bytes = valid.to_vec();
        bytes[20..22].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut case = repaired_case(engine, "unsupported_encoding_mode_65535".into(), bytes);
        case.binding = MustReject;
        case.native = MustReject;
        cases.push(case);
    }
    let start = if engine.is_dt() { 9 } else { 22 };
    for (index, payload) in [
        vec![255; 5],
        vec![255; 10],
        vec![128; 32],
        vec![128; 256],
        vec![255, 255, 255, 255, 15],
        vec![254, 255, 255, 255, 255, 255, 255, 255, 127],
    ]
    .into_iter()
    .enumerate()
    {
        for offset in [start, start + 8] {
            let end = if engine.is_dt() {
                valid.len() - 6
            } else {
                valid.len()
            };
            let mut bytes = valid[..end].to_vec();
            let stop = (offset + payload.len()).min(bytes.len());
            bytes.splice(offset..stop, payload.iter().copied());
            let mut case = repaired_case(
                engine,
                format!("extreme_varint_length_{offset}_{index}"),
                bytes,
            );
            case.risk = "high";
            cases.push(case);
        }
    }
    let mut random = 0x6a09_e667_f3bc_c909_u64;
    for index in 0..16 {
        let length = [0, 1, 2, 3, 7, 15, 31, 63, 127, 255, 1024, 4096][index % 12];
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            bytes.push(random.to_le_bytes()[0]);
        }
        cases.push(Case::observation(
            format!("seeded_random_{index}"),
            bytes.clone(),
            "not_applicable",
        ));
        let mut framed = valid[..start].to_vec();
        framed.extend(bytes);
        let mut case = repaired_case(engine, format!("seeded_framed_random_{index}"), framed);
        case.risk = "high";
        cases.push(case);
    }
    if cases.len() > MAX_CASES || cases.iter().any(|case| case.bytes.len() > MAX_ENCODED) {
        return Err(format!(
            "corpus exceeds bounded contract: {} cases; small update {} bytes",
            cases.len(),
            valid.len()
        ));
    }
    Ok(cases)
}

fn mutation_pair(cases: &mut Vec<Case>, engine: Engine, name: String, bytes: Vec<u8>) {
    cases.push(Case::observation(
        format!("{name}_raw"),
        bytes.clone(),
        "unrepaired",
    ));
    // DT mutations replace, rather than duplicate, the original final CRC.
    let prefix = if engine.is_dt() {
        bytes[..bytes.len() - 6].to_vec()
    } else {
        bytes
    };
    cases.push(repaired_case(engine, format!("{name}_sealed"), prefix));
}

fn repaired_case(engine: Engine, name: String, mut bytes: Vec<u8>) -> Case {
    if engine.is_dt() {
        // Callers supply an explicit prefix without the original CRC trailer.
        Case::observation(name, dt_seal(bytes), "crc32c_repaired")
    } else {
        if bytes.len() >= 22 {
            let checksum = xxh32(&bytes[20..], u32::from_le_bytes(*b"LORO"));
            bytes[16..20].copy_from_slice(&checksum.to_le_bytes());
        }
        Case::observation(name, bytes, "xxh32_repaired")
    }
}

// Independent fixture CRC-32C arithmetic from the existing strict.rs controls.
fn dt_seal(mut prefix: Vec<u8>) -> Vec<u8> {
    let mut crc = u32::MAX;
    for &byte in &prefix {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 0 { 0 } else { 0x82f6_3b78 };
        }
    }
    prefix.extend([100, 4]);
    prefix.extend((!crc).to_le_bytes());
    prefix
}

// Fixture checksum only, NOT a decoder. Algorithm checked against xxhash-rust
// 0.8.19 src/xxh32{,_common}.rs. Loro 1.17.1 src/encoding.rs uses seed b"LORO",
// body [20..], LE checksum [16..20] for current FastUpdates/FastSnapshot modes.
fn xxh32(mut bytes: &[u8], seed: u32) -> u32 {
    const P1: u32 = 0x9e37_79b1;
    const P2: u32 = 0x85eb_ca77;
    const P3: u32 = 0xc2b2_ae3d;
    const P4: u32 = 0x27d4_eb2f;
    const P5: u32 = 0x1656_67b1;
    let mut hash = u32::try_from(bytes.len()).expect("bounded fixture checksum");
    if bytes.len() >= 16 {
        let mut lanes = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        while bytes.len() >= 16 {
            for (lane, word) in lanes.iter_mut().zip(bytes[..16].as_chunks::<4>().0) {
                let value = u32::from_le_bytes(*word);
                *lane = lane
                    .wrapping_add(value.wrapping_mul(P2))
                    .rotate_left(13)
                    .wrapping_mul(P1);
            }
            bytes = &bytes[16..];
        }
        for (lane, rotation) in lanes.into_iter().zip([1, 7, 12, 18]) {
            hash = hash.wrapping_add(lane.rotate_left(rotation));
        }
    } else {
        hash = hash.wrapping_add(seed.wrapping_add(P5));
    }
    while bytes.len() >= 4 {
        let word = u32::from_le_bytes(bytes[..4].try_into().expect("four-byte checksum word"));
        hash = hash
            .wrapping_add(word.wrapping_mul(P3))
            .rotate_left(17)
            .wrapping_mul(P4);
        bytes = &bytes[4..];
    }
    for &byte in bytes {
        hash = hash
            .wrapping_add(u32::from(byte).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
    }
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(P2);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(P3);
    hash ^ (hash >> 16)
}

fn verify_loro_checksum(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 22
        || bytes[16..20] != xxh32(&bytes[20..], u32::from_le_bytes(*b"LORO")).to_le_bytes()
    {
        return Err("independent XXH32 differs from native exported fixture".into());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing into String");
    }
    output
}

fn trial(
    engine: Engine,
    index: usize,
    fixture: &mut Fixture,
    case: &Case,
    valid: &[u8],
) -> Result<bool, String> {
    let before = fixture.snapshot()?;
    let seed = fixture.seed_bytes()?;
    // Single process = single trial. Keep default panic hook for diagnostics and
    // flush exact input before crossing the native trust boundary.
    {
        let mut stderr = io::stderr().lock();
        writeln!(stderr, "SEED engine={} hex={}", engine.name(), hex(&seed)).map_err(error)?;
        writeln!(stderr, "VALID engine={} hex={}", engine.name(), hex(valid)).map_err(error)?;
        writeln!(
            stderr,
            "TRIAL engine={} index={} family={} hex={}",
            engine.name(),
            index,
            case.family,
            hex(&case.bytes)
        )
        .map_err(error)?;
        stderr.flush().map_err(error)?;
    }
    let start = Instant::now();
    let attempt = catch_unwind(AssertUnwindSafe(|| fixture.attempt(engine, &case.bytes)));
    let (accepts, errors, panics) = match attempt {
        Ok(Ok(())) => (1, 0, 0),
        Ok(Err(message)) => {
            eprintln!("ERROR {message}");
            (0, 1, 0)
        }
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic payload");
            eprintln!("PANIC {detail}");
            (0, 0, 1)
        }
    };
    eprintln!("ATTEMPT_DONE accepts={accepts} errors={errors} panics={panics}");
    let after = catch_unwind(AssertUnwindSafe(|| fixture.snapshot()));
    let (base_mutated, snapshot_panics) = match after {
        Ok(Ok(after)) => (if before == after { "0" } else { "1" }, 0),
        Ok(Err(message)) => {
            eprintln!("SNAPSHOT_ERROR {message}");
            // Unknown is not silently reported as an unchanged base.
            ("unknown", 0)
        }
        Err(_) => {
            eprintln!("SNAPSHOT_PANIC base state is unknown");
            ("unknown", 1)
        }
    };
    let expectation = case.expectation(engine);
    let control_failed = match expectation {
        Expectation::MustAccept => accepts != 1,
        Expectation::MustReject => errors != 1,
        Expectation::Observe => false,
    };
    let violation = control_failed
        || panics != 0
        || snapshot_panics != 0
        || base_mutated == "unknown"
        || (base_mutated == "1" && (engine.is_binding() || accepts == 0));
    println!(
        "{},{},{},{},{},{},{},{},1,{},{},{},{},{},{},{}",
        engine.name(),
        engine.layer(),
        index,
        case.family,
        expectation.name(),
        case.risk,
        case.checksum,
        case.bytes.len(),
        errors,
        accepts,
        panics,
        base_mutated,
        snapshot_panics,
        u8::from(violation),
        start.elapsed().as_micros()
    );
    io::stdout().flush().map_err(error)?;
    Ok(!violation)
}

fn run() -> Result<bool, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: screen {dt-binding|dt-native|loro-binding|loro-native} {list|CASE_INDEX}; run children under a parent watchdog and OS resource limits".into());
    }
    let engine = Engine::parse(&args[0])?;
    let (mut fixture, valid, controls) = fixtures(engine)?;
    let cases = corpus(engine, &valid, controls)?;
    if args[1] == "list" {
        println!("case_index,family,expectation,risk,checksum,encoded_bytes");
        for (index, case) in cases.iter().enumerate() {
            println!(
                "{index},{},{},{},{},{}",
                case.family,
                case.expectation(engine).name(),
                case.risk,
                case.checksum,
                case.bytes.len()
            );
        }
        Ok(true)
    } else {
        let index: usize = args[1].parse().map_err(error)?;
        let case = cases
            .get(index)
            .ok_or_else(|| format!("case index {index} out of range 0..{}", cases.len()))?;
        trial(engine, index, &mut fixture, case, &valid)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("SETUP_ERROR {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_deterministic_bounded_and_covers_every_small_truncation() {
        for engine in [Engine::DtBinding, Engine::LoroBinding] {
            let (_, valid, controls) = fixtures(engine).unwrap();
            let first = corpus(engine, &valid, controls).unwrap();
            let (_, second_valid, controls) = fixtures(engine).unwrap();
            let second = corpus(engine, &second_valid, controls).unwrap();
            assert_eq!(valid, second_valid);
            assert_eq!(first, second);
            assert!(first.len() <= MAX_CASES);
            assert!(first.iter().all(|case| case.bytes.len() <= MAX_ENCODED));
            for offset in 0..valid.len() {
                let case = first
                    .iter()
                    .find(|case| case.family == format!("truncate_{offset}"))
                    .unwrap();
                assert_eq!(case.bytes, valid[..offset]);
            }
        }
    }

    #[test]
    fn binding_must_reject_controls_and_valid_unicode_controls() {
        for engine in [Engine::DtBinding, Engine::LoroBinding] {
            let (mut fixture, valid, controls) = fixtures(engine).unwrap();
            let before = fixture.snapshot().unwrap();
            for case in corpus(engine, &valid, controls).unwrap() {
                // Resource probes and unconstrained mutations belong in isolated
                // children, not the ordinary in-process control tests.
                if case.risk == "high" || case.binding == Expectation::Observe {
                    continue;
                }
                let result = fixture.attempt(engine, &case.bytes);
                match case.binding {
                    Expectation::MustAccept => {
                        assert!(result.is_ok(), "{}: {result:?}", case.family)
                    }
                    Expectation::MustReject => assert!(result.is_err(), "{}", case.family),
                    Expectation::Observe => {}
                }
                assert_eq!(fixture.snapshot().unwrap(), before, "{}", case.family);
            }
        }
    }

    #[test]
    fn fixture_checksums_match_native_exports_and_known_vector() {
        assert_eq!(xxh32(b"", 0), 0x02cc_5d05);
        assert_eq!(xxh32(b"a", 0), 0x550d_7456);
        let (_, valid, _) = fixtures(Engine::DtBinding).unwrap();
        assert_eq!(dt_seal(valid[..valid.len() - 6].to_vec()), valid);
        let (_, valid, _) = fixtures(Engine::LoroBinding).unwrap();
        verify_loro_checksum(&valid).unwrap();
    }
}
