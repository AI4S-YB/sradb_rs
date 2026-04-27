//! Benchmark for the `EXPERIMENT_PACKAGE_SET` XML parser.
//!
//! Replicates the captured fixture N times to approximate a 1k-experiment
//! payload, then measures the time to parse it.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use sradb_core::parse::experiment_package;

const REPEAT: usize = 50;

fn synth_payload() -> String {
    let path = sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_xml_SRP174132.xml");
    let body = std::fs::read_to_string(&path).expect("fixture present");
    // Strip the wrapping element so we can splice multiple inner blocks.
    let open = body
        .find("<EXPERIMENT_PACKAGE_SET")
        .expect("opening tag present");
    let after_open = body[open..]
        .find('>')
        .map(|i| open + i + 1)
        .expect("opening tag closes");
    let close = body
        .rfind("</EXPERIMENT_PACKAGE_SET>")
        .expect("closing tag present");
    let inner = &body[after_open..close];

    let mut out = String::with_capacity(body.len() * REPEAT);
    out.push_str(&body[..after_open]);
    for _ in 0..REPEAT {
        out.push_str(inner);
    }
    out.push_str("</EXPERIMENT_PACKAGE_SET>");
    out
}

fn bench(c: &mut Criterion) {
    let payload = synth_payload();
    let mut group = c.benchmark_group("parse_experiment_xml");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("repeat50", |b| {
        b.iter(|| {
            let parsed = experiment_package::parse(black_box(&payload)).unwrap();
            black_box(parsed)
        });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
