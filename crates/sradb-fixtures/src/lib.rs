//! Dev-only helpers shared between sradb-core and sradb-cli tests.

use std::path::PathBuf;

/// Path to the workspace root, computed at compile time from this crate's
/// `CARGO_MANIFEST_DIR` (= `<root>/crates/sradb-fixtures`).
#[must_use]
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Load a fixture file from `<workspace>/tests/data/<relative>`.
///
/// Panics if the file is missing — fixtures must be committed.
#[must_use]
pub fn load_fixture(relative: &str) -> Vec<u8> {
    let path = workspace_root().join("tests/data").join(relative);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()))
}

/// Same as `load_fixture` but as UTF-8 string.
#[must_use]
pub fn load_fixture_str(relative: &str) -> String {
    let bytes = load_fixture(relative);
    String::from_utf8(bytes).expect("fixture is utf-8")
}

pub use wiremock;

/// Spin up a `wiremock::MockServer` and return it. Caller must hold the
/// server for the test's lifetime — drop = stop.
pub async fn mock_server() -> wiremock::MockServer {
    wiremock::MockServer::start().await
}

/// Construct a `(ncbi_base, ena_base)` pair rooted at the same mock server but
/// under different path prefixes. Tests register mocks at `/eutils/...` and
/// `/ena/...` to disambiguate which backend the call should hit.
#[must_use]
pub fn split_base_urls(server_uri: &str) -> (String, String) {
    (format!("{server_uri}/eutils"), format!("{server_uri}/ena"))
}
