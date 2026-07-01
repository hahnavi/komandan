//! YAML parsing benchmark.
//!
//! Measures the cost of [`komandan_playbook::parser::parse_playbook`] over a
//! small constant playbook and a generated ~50-task medium playbook. Execution
//! is not measured here — see `playbook.rs`.

mod common;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");

    group.bench_function("small_playbook", |b| {
        b.iter(|| {
            let result = common::parse(black_box(common::SMALL_PLAYBOOK));
            black_box(result);
        });
    });

    let medium_yaml = common::medium_playbook();
    group.bench_function("medium_playbook_50_tasks", |b| {
        b.iter(|| {
            let result = common::parse(black_box(&medium_yaml));
            black_box(result);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
