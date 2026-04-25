# sradb-rs

Rust port of [pysradb](https://github.com/saketkc/pysradb): query NGS metadata from NCBI SRA, ENA, and GEO.

**Status:** early development. Slice 1 (foundation) complete. See `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` for the design and `docs/superpowers/plans/` for implementation plans.

## Quickstart (dev)

```bash
cargo build --workspace
cargo test --workspace
cargo run -p sradb-cli -- info
```

## Layout

- `crates/sradb-core/` — async library: types, HTTP client, parsers (per-slice).
- `crates/sradb-cli/` — `sradb` CLI binary.
- `crates/sradb-fixtures/` — dev-only test helpers.
- `tools/capture-fixtures/` — captures real-API responses for offline tests.
- `tests/data/` — committed response fixtures.
- `pysradb/` — original Python implementation, kept in tree for reference (gitignored).

## Configuration

Environment variables:

- `NCBI_API_KEY` — raises NCBI rate limit from 3rps to 10rps.
- `NCBI_EMAIL` — recommended by NCBI E-utils etiquette.
- `OPENAI_API_KEY` — required for `--enrich` (slice 7+).
- `OPENAI_BASE_URL` — override for any OpenAI-compatible endpoint.

## License

MIT.
