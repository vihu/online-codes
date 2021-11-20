use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;
use criterion::{criterion_group, criterion_main};
use online_codes::{decode_block, encode_data, next_block};

fn check_encode_decode(buf: Vec<u8>) -> Option<Vec<u8>> {
    let (mut encoder, mut decoder) = encode_data(buf);

    loop {
        match next_block(&mut encoder) {
            Some(block) => match decode_block(block, &mut decoder) {
                None => continue,
                Some(res) => return Some(res),
            },
            None => continue,
        }
    }
}

fn identity_roundtrip(size: usize) -> Option<Vec<u8>> {
    let random_bytes: Vec<u8> = (0..size).map(|_| rand::random::<u8>()).collect();
    check_encode_decode(random_bytes)
}

fn kb_range(c: &mut Criterion) {
    static KB: usize = 1024;

    let mut group = c.benchmark_group("kb_range");
    for size in [KB, 2 * KB, 4 * KB, 8 * KB, 16 * KB].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| identity_roundtrip(size))
        });
    }
    group.finish();
}

criterion_group!(benches, kb_range);
criterion_main!(benches);
