use std::time::{SystemTime, UNIX_EPOCH};
use crate::block::Block;

pub struct Blockchain {
    pub chain: Vec<Block>,
}

impl Blockchain {
    pub fn new() -> Self {
        let mut blockchain = Blockchain { chain: vec![] };
        blockchain.create_genesis_block();
        blockchain
    }

    fn create_genesis_block(&mut self) {
        let genesis_block = Block::new(0, Self::current_timestamp(), String::from("Genesis Block"), String::from("0"));
        self.chain.push(genesis_block);
    }

    fn current_timestamp() -> u128 {
        let start = SystemTime::now();
        start.duration_since(UNIX_EPOCH).unwrap().as_millis()
    }

    pub fn get_latest_block(&self) -> &Block {
        self.chain.last().unwrap()
    }

    pub fn add_block(&mut self, data: String) {
        let latest_block = self.get_latest_block();
        let new_block = Block::new(
            latest_block.index + 1,
            Self::current_timestamp(),
            data,
            latest_block.hash.clone(),
        );
        self.chain.push(new_block);
    }

    pub fn is_chain_valid(&self) -> bool {
        for i in 1..self.chain.len() {
            let current_block = &self.chain[i];
            let previous_block = &self.chain[i - 1];

            if current_block.hash != current_block.calculate_hash() {
                return false;
            }
            if current_block.previous_hash != previous_block.hash {
                return false;
            }
        }
        true
    }
}
