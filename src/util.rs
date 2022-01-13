use crate::types::{BlockIndex, CheckBlockId, StreamId};
use rand::distributions::{Distribution, Uniform, WeightedIndex};
use rand_core::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;
use std::cmp;
use std::collections::{HashMap, HashSet};

// TODO: optimize
pub fn xor_block(dest: &mut [u8], src: &[u8], block_size: usize) {
    for i in 0..block_size {
        dest[i] ^= src[i];
    }
}

pub fn xor_adjacent_blocks(
    target_block_index: BlockIndex,
    adjacent_blocks: &[BlockIndex],
    augmented_data: &mut [u8],
    block_size: usize,
) {
    for block_index in adjacent_blocks {
        if *block_index != target_block_index {
            for i in 0..block_size {
                augmented_data[target_block_index * block_size + i] ^=
                    augmented_data[block_index * block_size + i];
            }
        }
    }
}

// TODO: don't lose bits when combining the stream id and block id
pub fn seed_block_rng(stream_id: StreamId, check_block_id: CheckBlockId) -> Xoshiro256StarStar {
    // Make sure the seed is a good, even mix of 0's and 1's.
    Xoshiro256StarStar::seed_from_u64(check_block_id.wrapping_add(stream_id))
}

pub fn get_adjacent_blocks(
    check_block_id: CheckBlockId,
    stream_id: StreamId,
    degree_distribution: &WeightedIndex<f64>,
    num_blocks: usize,
) -> Vec<BlockIndex> {
    assert!(num_blocks > 1);
    let mut rng = seed_block_rng(stream_id, check_block_id);
    let degree = 1 + degree_distribution.sample(&mut rng);
    // we don't want the block id itself, because a block is not adjacent to itself
    sample_with_exclusive_repeats(&mut rng, num_blocks, degree, Some(check_block_id as usize))
}

pub fn sample_with_exclusive_repeats(
    rng: &mut Xoshiro256StarStar,
    high_exclusive: usize,
    num: usize,
    exclude: Option<usize>,
) -> Vec<usize> {
    let mut selected = HashSet::with_capacity(num);
    let distribution = Uniform::new(0, high_exclusive);
    let mut found = 0;
    // try to get either num or high_exclusive number of unique samples
    // whichever is lower
    // if the 'excluded' value is in this range, lower the bound by 1
    // if the lowest value is 'high exclusive'
    let limit = match exclude {
        Some(s) if s < high_exclusive => cmp::min(num, high_exclusive - 1),
        _ => cmp::min(num, high_exclusive),
    };
    while found < limit {
        let sample = distribution.sample(rng);
        match exclude {
            Some(s) if sample == s => continue,
            _ => {
                if selected.insert(sample) {
                    found += 1;
                }
            }
        }
    }
    selected.into_iter().collect()
}

pub fn seed_stream_rng(stream_id: StreamId) -> Xoshiro256StarStar {
    seed_block_rng(stream_id, 0)
}

pub fn num_aux_blocks(num_blocks: usize, epsilon: f64, q: usize) -> usize {
    (0.55_f64 * q as f64 * epsilon * num_blocks as f64).ceil() as usize
}

pub fn get_aux_block_adjacencies(
    stream_id: StreamId,
    num_blocks: usize,
    num_auxiliary_blocks: usize,
    q: usize,
) -> HashMap<BlockIndex, (usize, Vec<BlockIndex>)> {
    let mut mapping: HashMap<BlockIndex, (usize, Vec<BlockIndex>)> = HashMap::new();
    let mut rng = seed_stream_rng(stream_id);
    for i in 0..num_blocks {
        for aux_index in sample_with_exclusive_repeats(&mut rng, num_auxiliary_blocks, q, None) {
            // TODO: clean up a bit
            let (num, ids) = &mut mapping.entry(aux_index + num_blocks).or_default();
            *num += 1;
            ids.push(i);
        }
    }
    mapping
}

pub fn make_degree_distribution(epsilon: f64) -> WeightedIndex<f64> {
    // See section 3.2 of the Maymounkov-Mazières paper.
    let f = ((f64::ln(epsilon * epsilon / 4.0)) / f64::ln(1.0 - epsilon / 2.0)).ceil() as usize;
    let mut p = Vec::with_capacity(f);
    let p1 = 1.0 - ((1.0 + 1.0 / f as f64) / (1.0 + epsilon));
    p.push(p1);
    // Extracted unchanging constant from p_i's.
    let c = (1.0 - p1) * f as f64 / (f - 1) as f64;
    for i in 2..=f {
        p.push(c / (i * (i - 1)) as f64);
    }
    WeightedIndex::new(&p).expect("serious probability calculation error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_with_exclusive_repeats_test() {
        let mut rng = seed_stream_rng(0);
        let ans = sample_with_exclusive_repeats(&mut rng, 1, 3, None);
        println!("ans: {:?}", ans);
    }

    #[test]
    fn num_aux_blocks_test() {
        let q = 3;
        let epsilon = 0.01;
        let num_blocks = 10;
        let num_aux_blocks = (0.55_f64 * q as f64 * epsilon * num_blocks as f64).ceil() as usize;
        println!("num_aux_blocks: {:?}", num_aux_blocks);
    }
}
