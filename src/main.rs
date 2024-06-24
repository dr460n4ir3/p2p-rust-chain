use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[macro_use]
extern crate serde_derive;

use serde_json;
use log::{info, error};
use env_logger;
use sha2::{Sha256, Digest};

const BLOCKCHAIN_FILE: &str = "blockchain.json";
const DIFFICULTY_PREFIX: &str = "0"; // Lower difficulty for testing switch back to 0000 for production

#[derive(Clone, Debug)]
struct Peer {
    address: SocketAddr,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Blockchain {
    blocks: Vec<Block>,
}

impl Blockchain {
    fn new() -> Self {
        Self { blocks: vec![Block::genesis()] }
    }

    fn add_block(&mut self, block: Block) {
        info!("Block added: {:?}", block);
        self.blocks.push(block);
        self.save_to_file();
    }

    fn latest_block(&self) -> &Block {
        self.blocks.last().expect("Blockchain should have at least one block")
    }

    fn validate(&self) -> bool {
        for i in 1..self.blocks.len() {
            let current = &self.blocks[i];
            let previous = &self.blocks[i - 1];
            if current.previous_hash != previous.hash || current.hash != current.calculate_hash() {
                return false;
            }
        }
        true
    }

    fn merge(&mut self, other: Blockchain) {
        if other.blocks.len() > self.blocks.len() && other.validate() {
            self.blocks = other.blocks;
            info!("Blockchain updated from peer");
            self.save_to_file();
        }
    }

    fn save_to_file(&self) {
        let json = serde_json::to_string_pretty(&self).expect("Failed to serialize blockchain");
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).open(BLOCKCHAIN_FILE).expect("Failed to open blockchain file");
        file.write_all(json.as_bytes()).expect("Failed to write to blockchain file");
    }

    fn load_from_file() -> Self {
        let mut file = match File::open(BLOCKCHAIN_FILE) {
            Ok(file) => file,
            Err(_) => return Self::new(),
        };

        let mut json = String::new();
        file.read_to_string(&mut json).expect("Failed to read from blockchain file");
        serde_json::from_str(&json).expect("Failed to deserialize blockchain")
    }

    fn reload_from_file(&mut self) {
        *self = Blockchain::load_from_file();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Block {
    index: u32,
    transactions: Vec<Transaction>,
    previous_hash: String,
    hash: String,
    timestamp: u128,
    nonce: u64,
}

impl Block {
    fn genesis() -> Self {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let mut block = Self {
            index: 0,
            transactions: vec![],
            previous_hash: "0".to_string(),
            hash: String::new(),
            timestamp,
            nonce: 0,
        };
        block.hash = block.calculate_hash();
        block
    }

    fn new(index: u32, transactions: Vec<Transaction>, previous_hash: String) -> Self {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let mut block = Self {
            index,
            transactions,
            previous_hash,
            hash: String::new(),
            timestamp,
            nonce: 0,
        };
        block.hash = block.calculate_hash();
        block
    }

    fn calculate_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}{:?}{}{}{}", self.index, self.transactions, self.previous_hash, self.timestamp, self.nonce));
        format!("{:x}", hasher.finalize())
    }

    fn mine_block(&mut self, difficulty_prefix: &str) {
        while &self.hash[..difficulty_prefix.len()] != difficulty_prefix {
            self.nonce += 1;
            self.hash = self.calculate_hash();
            if self.nonce % 1000 == 0 {
                info!("Mining block {}: nonce {}", self.index, self.nonce);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Transaction {
    from: String,
    to: String,
    amount: u32,
}

#[derive(Debug)]
struct Node {
    address: SocketAddr,
    peers: Arc<Mutex<HashMap<SocketAddr, Peer>>>,
    blockchain: Arc<Mutex<Blockchain>>,
    pending_transactions: Arc<Mutex<Vec<Transaction>>>,
}

impl Node {
    fn new(address: SocketAddr) -> Arc<Self> {
        Arc::new(Self {
            address,
            peers: Arc::new(Mutex::new(HashMap::new())),
            blockchain: Arc::new(Mutex::new(Blockchain::load_from_file())),
            pending_transactions: Arc::new(Mutex::new(vec![])),
        })
    }

    fn add_peer(self: &Arc<Self>, peer_address: SocketAddr) {
        let peer = Peer { address: peer_address };
        self.peers.lock().unwrap().insert(peer_address, peer);
        info!("Added peer: {}", peer_address);
    }

    fn remove_peer(&self, peer_address: SocketAddr) {
        self.peers.lock().unwrap().remove(&peer_address);
        info!("Removed peer: {}", peer_address);
    }

    fn start(self: Arc<Self>) {
        env_logger::init();
        let listener = TcpListener::bind(self.address).unwrap();
        info!("Node started at {}", self.address);

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                    continue;
                }
            };
            let node = Arc::clone(&self);
            thread::spawn(move || {
                node.handle_connection(stream);
            });
        }
    }

    fn handle_connection(&self, mut stream: TcpStream) {
        let mut buffer = [0; 2048];
        let bytes_read = match stream.read(&mut buffer) {
            Ok(bytes) => bytes,
            Err(e) => {
                error!("Failed to read from stream: {}", e);
                return;
            }
        };
        let message = String::from_utf8_lossy(&buffer[..bytes_read]);
        info!("Received: {}", message);

        if message.starts_with("GET /blockchain") {
            let mut blockchain = self.blockchain.lock().unwrap();
            blockchain.reload_from_file(); // Reload the blockchain from the file before responding
            let response = match serde_json::to_string(&*blockchain) {
                Ok(json) => json,
                Err(e) => {
                    error!("Failed to serialize blockchain: {}", e);
                    return;
                }
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", response.len(), response);
            if let Err(e) = stream.write(response.as_bytes()) {
                error!("Failed to write to stream: {}", e);
            }
        } else if message.starts_with("POST /transaction") {
            let parts: Vec<&str> = message.splitn(2, "\r\n\r\n").collect();
            if parts.len() == 2 && !parts[1].is_empty() {
                info!("Request body: {}", parts[1]);
                match serde_json::from_str::<Transaction>(parts[1]) {
                    Ok(transaction) => {
                        let mut pending_transactions = self.pending_transactions.lock().unwrap();
                        pending_transactions.push(transaction);
                        let response = "HTTP/1.1 201 CREATED\r\n\r\n";
                        if let Err(e) = stream.write(response.as_bytes()) {
                            error!("Failed to write to stream: {}", e);
                        }
                    },
                    Err(e) => {
                        error!("Failed to parse JSON: {}", e);
                        let response = format!("HTTP/1.1 400 BAD REQUEST\r\n\r\nInvalid JSON body: {}", e);
                        if let Err(e) = stream.write(response.as_bytes()) {
                            error!("Failed to write to stream: {}", e);
                        }
                    }
                }
            } else {
                error!("Empty body");
                let response = "HTTP/1.1 400 BAD REQUEST\r\n\r\nEmpty body";
                if let Err(e) = stream.write(response.as_bytes()) {
                    error!("Failed to write to stream: {}", e);
                }
            }
        } else if message.starts_with("POST /mine") {
            let parts: Vec<&str> = message.splitn(2, "\r\n\r\n").collect();
            if parts.len() == 2 && !parts[1].is_empty() {
                info!("Request body: {}", parts[1]);
                match serde_json::from_str::<serde_json::Value>(parts[1]) {
                    Ok(json_value) => {
                        if let Some(miner) = json_value.get("miner") {
                            let miner_address = miner.as_str().unwrap().to_string();
                            let mut blockchain = self.blockchain.lock().unwrap();
                            let mut transactions = self.pending_transactions.lock().unwrap().clone();
                            transactions.push(Transaction {
                                from: "network".to_string(),
                                to: miner_address.clone(),
                                amount: 50, // Block reward
                            });
                            let mut new_block = Block::new(blockchain.blocks.len() as u32, transactions, blockchain.latest_block().hash.clone());
                            new_block.mine_block(DIFFICULTY_PREFIX);
                            blockchain.add_block(new_block);
                            let response = "HTTP/1.1 201 CREATED\r\n\r\n";
                            if let Err(e) = stream.write(response.as_bytes()) {
                                error!("Failed to write to stream: {}", e);
                            }
                        } else {
                            error!("Invalid JSON body: missing field `miner`");
                            let response = "HTTP/1.1 400 BAD REQUEST\r\n\r\nInvalid JSON body: missing field `miner`";
                            if let Err(e) = stream.write(response.as_bytes()) {
                                error!("Failed to write to stream: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        error!("Failed to parse JSON: {}", e);
                        let response = format!("HTTP/1.1 400 BAD REQUEST\r\n\r\nInvalid JSON body: {}", e);
                        if let Err(e) = stream.write(response.as_bytes()) {
                            error!("Failed to write to stream: {}", e);
                        }
                    }
                }
            } else {
                error!("Empty body");
                let response = "HTTP/1.1 400 BAD REQUEST\r\n\r\nEmpty body";
                if let Err(e) = stream.write(response.as_bytes()) {
                    error!("Failed to write to stream: {}", e);
                }
            }
        } else if message.starts_with("NEW BLOCK") {
            let parts: Vec<&str> = message.splitn(2, "\r\n\r\n").collect();
            if parts.len() == 2 {
                if let Ok(new_block) = serde_json::from_str(parts[1]) {
                    let mut blockchain = self.blockchain.lock().unwrap();
                    blockchain.reload_from_file(); // Reload the blockchain from the file before adding a new block
                    blockchain.add_block(new_block);
                }
            }
        } else if message.starts_with("POST /reset") {
            let mut blockchain = self.blockchain.lock().unwrap();
            *blockchain = Blockchain::new();
            blockchain.save_to_file();
            let response = "HTTP/1.1 200 OK\r\n\r\nBlockchain has been reset";
            if let Err(e) = stream.write(response.as_bytes()) {
                error!("Failed to write to stream: {}", e);
            }
        } else {
            let response = "HTTP/1.1 404 NOT FOUND\r\n\r\n";
            if let Err(e) = stream.write(response.as_bytes()) {
                error!("Failed to write to stream: {}", e);
            }
        }
    }

    fn broadcast(&self, message: String) {
        let peers = self.peers.lock().unwrap().clone();
        for peer in peers.values() {
            let address = peer.address;
            let message = message.clone();
            thread::spawn(move || {
                if let Ok(mut stream) = TcpStream::connect(address) {
                    if let Err(e) = stream.write(message.as_bytes()) {
                        error!("Failed to write to peer {}: {}", address, e);
                    }
                }
            });
        }
    }

    fn sync_with_peers(self: Arc<Self>) {
        let peers = self.peers.lock().unwrap().clone();
        for peer in peers.values() {
            let address = peer.address;
            let node_clone = Arc::clone(&self);
            thread::spawn(move || {
                if let Ok(mut stream) = TcpStream::connect(address) {
                    let request = "GET /blockchain HTTP/1.1\r\n\r\n";
                    if stream.write(request.as_bytes()).is_ok() {
                        let mut buffer = Vec::new();
                        if stream.read_to_end(&mut buffer).is_ok() {
                            if let Ok(peer_blockchain) = serde_json::from_slice::<Blockchain>(&buffer) {
                                let mut blockchain = node_clone.blockchain.lock().unwrap();
                                blockchain.reload_from_file(); // Reload the blockchain from the file before merging
                                blockchain.merge(peer_blockchain);
                            } else {
                                error!("Failed to parse blockchain from peer {}", address);
                            }
                        }
                    }
                }
            });
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <node address> [peer address]", args[0]);
        return;
    }

    let node_address: SocketAddr = args[1].parse().expect("Invalid node address");
    let node = Node::new(node_address);

    if args.len() > 2 {
        let peer_address: SocketAddr = args[2].parse().expect("Invalid peer address");
        node.add_peer(peer_address);
    }

    node.start();
}
