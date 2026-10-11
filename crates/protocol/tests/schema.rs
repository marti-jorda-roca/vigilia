//! Fails when `schema/protocol.schema.json` no longer matches the Rust types.
//! Regenerate with `UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema`.

use std::path::PathBuf;

#[test]
fn checked_in_schema_matches_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/protocol.schema.json");
    let generated = serde_json::to_string_pretty(&vigilia_protocol::schema()).unwrap() + "\n";

    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }

    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == generated,
        "{} is out of date. Run `UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema` \
         and commit the result. Remember: protocol changes may only add things.",
        path.display()
    );
}
