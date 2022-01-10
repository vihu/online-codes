use crate::types::{BlockIndex, CheckBlockId, StreamId};
use crate::util::{
    get_adjacent_blocks, get_aux_block_adjacencies, make_degree_distribution, num_aux_blocks,
    xor_adjacent_blocks, xor_block,
};
use rand::distributions::WeightedIndex;
use std::collections::{hash_map::Entry, HashMap};

#[derive(Debug)]
pub enum DecodeResult {
    Complete(Vec<u8>),
    InProgress(Box<Decoder>),
}

enum UndecodedDegree {
    Zero,
    One(BlockIndex), // id of single block which hasn't yet been decoded
    Many(usize),     // number of blocks that haven't yet been decoded
}

#[derive(Clone, Debug)]
pub struct Decoder {
    pub num_blocks: usize,
    pub num_augmented_blocks: usize,
    pub block_size: usize,
    pub degree_distribution: WeightedIndex<f64>,
    pub stream_id: StreamId,
    pub unused_aux_block_adjacencies: HashMap<BlockIndex, (usize, Vec<BlockIndex>)>,
    pub augmented_data: Vec<u8>,
    pub blocks_decoded: Vec<bool>,
    pub num_undecoded_data_blocks: usize,
    pub unused_check_blocks: HashMap<CheckBlockId, (usize, Vec<u8>)>,
    pub adjacent_check_blocks: HashMap<BlockIndex, Vec<CheckBlockId>>,
    pub decode_stack: Vec<(CheckBlockId, Vec<u8>)>,
    pub aux_decode_stack: Vec<(BlockIndex, Vec<BlockIndex>)>,
    pub pad: usize,
}

impl DecodeResult {
    pub fn complete(self) -> Option<Vec<u8>> {
        match self {
            DecodeResult::Complete(v) => Some(v),
            DecodeResult::InProgress(_) => None,
        }
    }
}

impl<'a> Decoder {
    pub fn new(num_blocks: usize, block_size: usize, stream_id: StreamId, pad: usize) -> Decoder {
        Self::with_parameters(num_blocks, block_size, stream_id, 0.01, 3, pad)
    }

    pub fn with_parameters(
        num_blocks: usize,
        block_size: usize,
        stream_id: StreamId,
        epsilon: f64,
        q: usize,
        pad: usize,
    ) -> Decoder {
        let num_aux_blocks = num_aux_blocks(num_blocks, epsilon, q);
        let num_augmented_blocks = num_blocks + num_aux_blocks;
        let unused_aux_block_adjacencies =
            get_aux_block_adjacencies(stream_id, num_blocks, num_aux_blocks, q);
        Decoder {
            num_blocks,
            num_augmented_blocks,
            block_size,
            unused_aux_block_adjacencies,
            degree_distribution: make_degree_distribution(epsilon),
            stream_id,
            augmented_data: vec![0; num_augmented_blocks * block_size],
            blocks_decoded: vec![false; num_augmented_blocks],
            num_undecoded_data_blocks: num_blocks,
            unused_check_blocks: HashMap::new(),
            adjacent_check_blocks: HashMap::new(),
            decode_stack: Vec::new(),
            aux_decode_stack: Vec::new(),
            pad,
        }
    }

    pub fn decode_block(
        &mut self,
        check_block_id: CheckBlockId,
        check_block: &[u8],
    ) -> Option<Vec<u8>> {
        // TODO: don't immediately push then pop off the decode stack
        self.decode_stack
            .push((check_block_id, check_block.to_owned()));

        self.process_decode_stack();
        self.process_aux_decode_stack();

        if self.num_undecoded_data_blocks == 0 {
            // Decoding finished -- return decoded data.
            let mut decoded_data = self.augmented_data.clone();
            decoded_data.truncate(self.block_size * self.num_blocks);
            Some(decoded_data)
        } else {
            // Decoding not yet complete.
            None
        }
    }

    pub fn into_iter<T>(mut self, iter: T) -> DecodeResult
    where
        T: IntoIterator<Item = (CheckBlockId, &'a [u8])>,
    {
        for (check_block_id, check_block) in iter {
            if let Some(decoded_data) = self.decode_block(check_block_id, check_block) {
                return DecodeResult::Complete(decoded_data);
            }
        }
        DecodeResult::InProgress(Box::new(self))
    }

    pub fn get_incomplete_result(&self) -> (&[bool], &[u8]) {
        (
            &self.blocks_decoded[0..self.num_blocks],
            &self.augmented_data[0..self.block_size * self.num_blocks],
        )
    }

    pub fn into_incomplete_result(mut self) -> (Vec<bool>, Vec<u8>) {
        self.blocks_decoded.truncate(self.num_blocks);
        self.augmented_data
            .truncate(self.num_blocks * self.block_size);
        (self.blocks_decoded, self.augmented_data)
    }

    fn decode_data_block_from_aux_block(&mut self, target_block_index: usize) {
        if let Entry::Occupied(mut unused_aux_entry) =
            self.unused_aux_block_adjacencies.entry(target_block_index)
        {
            let remaining_degree = &mut unused_aux_entry.get_mut().0;
            *remaining_degree -= 1;
            if *remaining_degree == 1 {
                self.aux_decode_stack
                    .push((target_block_index, unused_aux_entry.remove().1));
            }
        }
    }

    fn handle_degree_many(
        &mut self,
        degree: usize,
        check_block_id: CheckBlockId,
        check_block: &[u8],
        adjacent_blocks: Vec<BlockIndex>,
    ) {
        self.unused_check_blocks
            .insert(check_block_id, (degree, check_block.to_owned()));
        for block_index in adjacent_blocks {
            self.adjacent_check_blocks
                .entry(block_index)
                .or_default()
                .push(check_block_id)
        }
    }

    fn handle_degree_one(
        &mut self,
        target_block_index: usize,
        check_block: &[u8],
        adjacent_blocks: Vec<BlockIndex>,
    ) {
        self.decode_from_check_block(target_block_index, check_block, &adjacent_blocks);
        self.blocks_decoded[target_block_index] = true;
        if target_block_index < self.num_blocks {
            self.num_undecoded_data_blocks -= 1;
        } else {
            self.decode_data_block_from_aux_block(target_block_index);
        }
        if let Some(adjacent_check_block_ids) =
            self.adjacent_check_blocks.remove(&target_block_index)
        {
            for check_block_id in adjacent_check_block_ids {
                if let Entry::Occupied(mut unused_block_entry) =
                    self.unused_check_blocks.entry(check_block_id)
                {
                    let remaining_degree = &mut unused_block_entry.get_mut().0;
                    *remaining_degree -= 1;
                    if *remaining_degree == 1 {
                        self.decode_stack
                            .push((check_block_id, unused_block_entry.remove().1.to_owned()));
                    }
                }
            }
        };
    }

    fn process_decode_stack(&mut self) {
        while let Some((check_block_id, check_block)) = self.decode_stack.pop() {
            let adjacent_blocks = get_adjacent_blocks(
                check_block_id,
                self.stream_id,
                &self.degree_distribution,
                self.num_augmented_blocks,
            );
            match self.undecoded_degree(&adjacent_blocks) {
                UndecodedDegree::Zero => (), // nothing to do here
                UndecodedDegree::One(target_block_index) => {
                    self.handle_degree_one(target_block_index, &check_block, adjacent_blocks)
                }
                UndecodedDegree::Many(degree) => {
                    self.handle_degree_many(degree, check_block_id, &check_block, adjacent_blocks);
                }
            }
        }
    }

    fn process_aux_decode_stack(&mut self) {
        while let Some((aux_block_index, adjacent_blocks)) = self.aux_decode_stack.pop() {
            if let Some(decoded_block_id) = self.decode_aux_block(aux_block_index, &adjacent_blocks)
            {
                self.blocks_decoded[decoded_block_id] = true;
                self.num_undecoded_data_blocks -= 1;
            }
        }
    }

    fn decode_from_check_block(
        &mut self,
        target_block_index: BlockIndex,
        check_block: &[u8],
        adjacent_blocks: &[BlockIndex],
    ) {
        xor_block(
            &mut self.augmented_data[target_block_index * self.block_size..],
            check_block,
            self.block_size,
        );
        xor_adjacent_blocks(
            target_block_index,
            adjacent_blocks,
            &mut self.augmented_data,
            self.block_size,
        );
    }

    fn decode_aux_block(
        &mut self,
        index: BlockIndex,
        adjacent_blocks: &[BlockIndex],
    ) -> Option<BlockIndex> {
        self.block_to_decode(adjacent_blocks)
            .map(|target_block_index| {
                for i in 0..self.block_size {
                    self.augmented_data[target_block_index * self.block_size + i] ^=
                        self.augmented_data[index * self.block_size + i];
                }
                xor_adjacent_blocks(
                    target_block_index,
                    adjacent_blocks,
                    &mut self.augmented_data,
                    self.block_size,
                );
                target_block_index
            })
    }

    fn undecoded_degree(&self, adjacent_block_ids: &[BlockIndex]) -> UndecodedDegree {
        // If exactly one of the adjacent blocks is not yet decoded, return the id of that block.
        let mut degree = UndecodedDegree::Zero;
        for block_index in adjacent_block_ids {
            if !self.blocks_decoded[*block_index] {
                degree = match degree {
                    UndecodedDegree::Zero => UndecodedDegree::One(*block_index),
                    UndecodedDegree::One(n) => UndecodedDegree::Many(n),
                    UndecodedDegree::Many(n) => UndecodedDegree::Many(n + 1),
                }
            }
        }

        degree
    }

    fn block_to_decode(&self, adjacent_blocks: &[BlockIndex]) -> Option<BlockIndex> {
        // If exactly one of the adjacent blocks is not yet decoded, return the id of that block.
        let mut to_decode = None;
        for block_index in adjacent_blocks {
            if !self.blocks_decoded[*block_index] {
                if to_decode.is_some() {
                    return None;
                }
                to_decode = Some(*block_index)
            }
        }

        to_decode
    }
}
