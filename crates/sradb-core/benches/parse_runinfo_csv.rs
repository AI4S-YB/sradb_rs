//! Benchmark for the runinfo CSV parser. Generates a synthetic 10k-row table
//! whose layout matches the captured fixture header.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use sradb_core::parse::runinfo;

const ROWS: usize = 10_000;

fn synth_csv() -> String {
    let header = "Run,ReleaseDate,LoadDate,spots,bases,spots_with_mates,avgLength,size_MB,\
        AssemblyName,download_path,Experiment,LibraryName,LibraryStrategy,LibrarySelection,\
        LibrarySource,LibraryLayout,InsertSize,InsertDev,Platform,Model,SRAStudy,BioProject,\
        Study_Pubmed_id,ProjectID,Sample,BioSample,SampleType,TaxID,ScientificName,SampleName,\
        g1k_pop_code,source,g1k_analysis_group,Subject_ID,Sex,Disease,Tumor,Affection_Status,\
        Analyte_Type,Histological_Type,Body_Site,CenterName,Submission,dbgap_study_accession,\
        Consent,RunHash,ReadHash";
    let mut out = String::with_capacity(ROWS * 256);
    out.push_str(header);
    out.push('\n');
    for i in 0..ROWS {
        // 47 columns; values realistic enough to exercise the parser.
        let line = format!(
            "SRR{i:07},2019-01-01,2019-01-02,{spots},{bases},0,150,512,,\
https://example/SRR{i:07}.sra,SRX{i:07},,,RNA-Seq,cDNA,TRANSCRIPTOMIC,PAIRED,300,30,ILLUMINA,\
Illumina HiSeq 2000,SRP174132,PRJNA511021,,123,SRS{i:07},SAMN10621{i:03},simple,9606,Homo sapiens,\
sample{i},,,,Subj{i},male,no,no,affected,DNA,,,GEO,SRA826111,phs0,public,,,",
            i = i,
            spots = 38_671_668 + i,
            bases = 11_678_843_736u64 + i as u64
        );
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn bench(c: &mut Criterion) {
    let payload = synth_csv();
    let mut group = c.benchmark_group("parse_runinfo_csv");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("rows10k", |b| {
        b.iter(|| {
            let parsed = runinfo::parse(black_box(&payload)).unwrap();
            black_box(parsed)
        });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
