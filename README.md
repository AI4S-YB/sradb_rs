# sradb-rs

A Rust port of [pysradb](https://github.com/saketkc/pysradb) — query and download NGS metadata and data from NCBI SRA, ENA, and GEO.

## Status

Slices 1–10 complete. Feature-equivalent with pysradb's CLI for the most-used workflows.

## Installation

```bash
cargo install --path crates/sradb-cli
```

Requires Rust 1.80+.

## CLI surface

```bash
sradb metadata <ACCESSION>... [--detailed] [--enrich] [--format tsv|json|ndjson]
sradb convert <FROM> <TO> <ACCESSION>...
sradb search [--query ...] [--organism ...] [--strategy ...] [--platform ...]
sradb download <ACCESSION>... [--source ncbi|ncbi-lite|ngdc|ena] [--out-dir DIR] [-j N] [--dry]
sradb geo matrix <GSE> [--out-dir DIR] [--parse-tsv]
sradb id <PMID|DOI|PMC> [--json]
sradb info
```

### Examples

Fetch metadata as TSV:
```bash
sradb metadata SRP174132 --format tsv
```

Get full detail with sample attributes and ENA fastq URLs:
```bash
sradb metadata SRP174132 --detailed --format json
```

Enrich with LLM-extracted ontology fields (organ / tissue / cell_type / etc.):
```bash
export OPENAI_API_KEY=sk-...
sradb metadata SRP174132 --detailed --enrich --format json
```

Convert between accession kinds:
```bash
sradb convert srp srx SRP174132     # → 10 SRX accessions
sradb convert gse srp GSE56924      # → SRP041298
sradb convert gsm srp GSM1371490
```

Search SRA:
```bash
sradb search --organism "Homo sapiens" --strategy RNA-Seq --max 10 --format json
```

Download full NCBI SRA files in parallel (default source):
```bash
sradb download SRP174132 --out-dir ./sra -j 4
```
Downloads are resumable: existing `.part` files are continued with HTTP `Range`, and the progress display shows one summary line plus one progress line per file with bytes, speed, ETA, retry count, and resumed bytes.

Print the resolved download URLs without downloading:
```bash
sradb download SRP174132 --dry
```

Download NCBI SRA Lite files explicitly:
```bash
sradb download SRP174132 --source ncbi-lite --out-dir ./sra-lite -j 4
```

Download full SRA files from the CNCB-NGDC mirror. The command resolves the NGDC browse page for each run and uses the HTTP URL published there:
```bash
sradb download SRP174132 --source ngdc --out-dir ./ngdc-sra -j 4
```

Download ENA FASTQ files instead:
```bash
sradb download SRP174132 --source ena --out-dir ./fastq -j 4
```

Download a GEO Series Matrix:
```bash
sradb geo matrix GSE56924 --out-dir ./geo --parse-tsv
```

Extract identifiers (GSE / GSM / SRP / PRJNA) from a publication:
```bash
sradb id 39528918 --json           # PMID
sradb id PMC10802650 --json        # PMC
sradb id 10.12688/f1000research.18676.1 --json   # DOI
```

## Configuration

Environment variables:

| Variable | Purpose |
| --- | --- |
| `NCBI_API_KEY` | Raises NCBI rate limit from 3 rps to 10 rps |
| `NCBI_EMAIL` | Recommended by NCBI eUtils etiquette |
| `OPENAI_API_KEY` | Required for `--enrich` |
| `OPENAI_BASE_URL` | Override (default `https://api.openai.com`) — works with any OpenAI-compatible endpoint (Azure, Together, vLLM, llama.cpp server, Ollama's `/v1` endpoint) |
| `OPENAI_MODEL` | Override (default `gpt-4o-mini`) |

## Architecture

Cargo workspace with three crates:

- `crates/sradb-core/` — async library: types, HTTP client, parsers, orchestrator
- `crates/sradb-cli/` — `sradb` binary (clap-based CLI)
- `crates/sradb-fixtures/` — dev-only test helpers

Plus a dev tool `tools/capture-fixtures/` for capturing real-API responses for offline tests.

## Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

For development, the original Python `pysradb/` is kept in tree as reference (gitignored).

## Release

Current release target: `v0.3.0`.

Release order is documented in [docs/release.md](docs/release.md): publish the GitHub Release first, then let the release workflow build and upload the binary archives.

## Testing strategy

- **Unit tests** — pure functions, no I/O.
- **Recorded-fixture tests** — wiremock stub server replays captured XML/JSON/TSV from `tests/data/`.
- **Live tests** — gated behind `--features live`, run manually before releases.
- **Property tests** — `proptest` for the accession parser round-trip.

## License

MIT.
