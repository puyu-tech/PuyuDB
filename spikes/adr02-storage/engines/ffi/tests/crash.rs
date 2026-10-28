#[test]
fn publication_crash_matrix() {
    adr02_protocol::crash_matrix(&adr02_protocol::engine_bin("adr02-ffi"));
}
