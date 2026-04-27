//! Benchmark for the pipe-delimited `sample_attribute` parser.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use sradb_core::parse::sample_attrs;

const STRINGS: usize = 1_000;

fn synth_inputs() -> Vec<String> {
    (0..STRINGS)
        .map(|i| {
            format!(
                "source_name: liver_{i} || \
cell type: hepatocyte || \
disease: control || \
isolate: subject_{i} || \
tissue: liver:adult || \
biomaterial provider: lab_{i} || \
collection date: 2019-01-{day:02}",
                i = i,
                day = (i % 28) + 1
            )
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let inputs = synth_inputs();
    let total_bytes: u64 = inputs.iter().map(|s| s.len() as u64).sum();
    let mut group = c.benchmark_group("parse_sample_attrs");
    group.throughput(Throughput::Bytes(total_bytes));
    group.bench_function("strings1k", |b| {
        b.iter(|| {
            for s in &inputs {
                let parsed = sample_attrs::parse(black_box(s));
                black_box(parsed);
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
