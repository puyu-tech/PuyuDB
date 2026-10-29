#[test]
fn publication_crash_matrix() {
    adr02_protocol::crash_matrix(&adr02_protocol::engine_bin("adr02-rusqlite"));
}

#[test]
fn pre_ack_creation_kills() {
    adr02_protocol::pre_ack_kills(&adr02_protocol::engine_bin("adr02-rusqlite"));
}
