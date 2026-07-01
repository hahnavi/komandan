//! Execution benchmark.
//!
//! Measures the full [`komandan_playbook::runner::execute`] path — play/host
//! orchestration, templating, loop/conditional evaluation, handler flushing —
//! against the mock `CoreApi` ([`null_core`], no real host contact). Parsing is
//! done once in setup and excluded from the measured iteration.

mod common;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_execute(c: &mut Criterion) {
    // Parse once (setup — not measured).
    let small = common::parse(common::SMALL_PLAYBOOK);
    let medium_yaml = common::medium_playbook();
    let medium = common::parse(&medium_yaml);

    let mut group = c.benchmark_group("execute");

    group.bench_function("small_playbook_forks_1", |b| {
        b.iter(|| {
            let result = common::run_parsed(black_box(&small));
            black_box(result);
        });
    });

    group.bench_function("medium_playbook_forks_1", |b| {
        b.iter(|| {
            let result = common::run_parsed(black_box(&medium));
            black_box(result);
        });
    });

    group.bench_function("small_playbook_forks_5", |b| {
        b.iter(|| {
            let result = common::run_parsed_with_forks(black_box(&small), 5);
            black_box(result);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_execute);
criterion_main!(benches);
