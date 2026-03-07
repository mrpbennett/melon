use melon::completion::engine::CompletionEngine;
use melon::completion::loader::SpecStore;
use melon::completion::matcher::FuzzyMatcher;
use melon::completion::source::{CompletionContext, CompletionSource, PathSource};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn main() {
    let iterations = std::env::var("MELON_BENCH_ITERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);

    println!("melon perf bench");
    println!("iterations={iterations}");

    bench_completion(iterations);
    bench_popup_typing(iterations / 4);
    bench_path_cache(iterations / 2);
}

fn bench_completion(iterations: usize) {
    let mut store = SpecStore::new();
    store.load_builtin();
    let mut engine = CompletionEngine::new(store);
    let mut matcher = FuzzyMatcher::new();

    run_bench("completion.git_com", iterations, || {
        let completion = engine.complete("git com", ".");
        black_box(matcher.filter(&completion.partial, completion.candidates));
    });
}

fn bench_popup_typing(iterations: usize) {
    let mut store = SpecStore::new();
    store.load_builtin();
    let mut engine = CompletionEngine::new(store);
    let mut matcher = FuzzyMatcher::new();
    let inputs = [
        "git c",
        "git co",
        "git com",
        "git comm",
        "git commi",
        "git commit",
    ];

    run_bench("completion.popup_typing", iterations, || {
        for input in &inputs {
            let completion = engine.complete(input, ".");
            black_box(matcher.filter(&completion.partial, completion.candidates));
        }
    });
}

fn bench_path_cache(iterations: usize) {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    populate_dir(dir_a.path(), 400);
    populate_dir(dir_b.path(), 400);

    let cached_context = CompletionContext {
        command: "git".into(),
        subcommands: vec!["add".into()],
        partial: format!("{}/fi", dir_a.path().display()),
        completing_option_arg: false,
        cwd: ".".into(),
    };
    let alternating_a = CompletionContext {
        command: "git".into(),
        subcommands: vec!["add".into()],
        partial: format!("{}/fi", dir_a.path().display()),
        completing_option_arg: false,
        cwd: ".".into(),
    };
    let alternating_b = CompletionContext {
        command: "git".into(),
        subcommands: vec!["add".into()],
        partial: format!("{}/fi", dir_b.path().display()),
        completing_option_arg: false,
        cwd: ".".into(),
    };

    let mut cached_source = PathSource::default();
    run_bench("path.cached_same_base", iterations, || {
        black_box(cached_source.candidates(&cached_context));
    });

    let mut alternating_source = PathSource::default();
    let mut use_a = false;
    run_bench("path.alternating_base", iterations, || {
        use_a = !use_a;
        let context = if use_a {
            &alternating_a
        } else {
            &alternating_b
        };
        black_box(alternating_source.candidates(context));
    });
}

fn populate_dir(path: &std::path::Path, files: usize) {
    for index in 0..files {
        let file_name = format!("file-{index:04}.txt");
        std::fs::write(path.join(file_name), "x").unwrap();
    }
}

fn run_bench(name: &str, iterations: usize, mut f: impl FnMut()) {
    let warmup_iterations = iterations.min(1_000);
    for _ in 0..warmup_iterations {
        f();
    }

    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();
    print_result(name, iterations, elapsed);
}

fn print_result(name: &str, iterations: usize, elapsed: Duration) {
    let total_ms = elapsed.as_secs_f64() * 1_000.0;
    let avg_us = elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64;
    println!("{name:24} total={total_ms:8.2}ms avg={avg_us:8.2}us");
}
