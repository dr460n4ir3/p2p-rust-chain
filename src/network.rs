use futures::prelude::*;
use libp2p::{
    core::upgrade,
    floodsub::{self, Floodsub, FloodsubEvent, Topic},
    identity,
    mdns::{Mdns, MdnsConfig, MdnsEvent},
    mplex,
    noise::{Keypair, NoiseConfig, X25519Spec},
    swarm::{SwarmBuilder, SwarmEvent},
    tcp::TcpConfig,
    yamux::YamuxConfig,
    PeerId, Swarm, Transport,
};
use serde::{Deserialize, Serialize};
use crate::blockchain::Blockchain;
use crate::block::Block;

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    NewBlock(Block),
    RequestChain,
    ResponseChain(Vec<Block>),
}

pub async fn start_network(mut blockchain: Blockchain) {
    // Generate a keypair for encryption
    let id_keys = Keypair::<X25519Spec>::new().into_authentic(&identity::Keypair::generate_ed25519()).unwrap();
    let local_peer_id = PeerId::from(id_keys.public());

    // Set up the transport stack
    let transport = TcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(id_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    // Set up the floodsub protocol
    let floodsub_topic = Topic::new("blocks");
    let mut floodsub = Floodsub::new(local_peer_id.clone());
    floodsub.subscribe(floodsub_topic.clone());

    // Set up mDNS for local network peer discovery
    let mdns = Mdns::new(MdnsConfig::default()).await.unwrap();

    // Build the swarm
    let mut swarm = {
        let behaviour = MyBehaviour {
            floodsub,
            mdns,
            blockchain,
        };
        SwarmBuilder::new(transport, behaviour, local_peer_id.clone())
            .executor(Box::new(|fut| {
                tokio::spawn(fut);
            }))
            .build()
    };

    // Start the swarm
    loop {
        match swarm.next().await.unwrap() {
            SwarmEvent::Behaviour(MyEvent::Floodsub(FloodsubEvent::Message(message))) => {
                if let Ok(msg) = serde_json::from_slice::<Message>(&message.data) {
                    handle_message(msg, &mut swarm.behaviour_mut().blockchain).await;
                }
            }
            SwarmEvent::Behaviour(MyEvent::Mdns(MdnsEvent::Discovered(peers))) => {
                for (peer_id, _addr) in peers {
                    swarm.behaviour_mut().floodsub.add_node_to_partial_view(peer_id);
                }
            }
            SwarmEvent::Behaviour(MyEvent::Mdns(MdnsEvent::Expired(peers))) => {
                for (peer_id, _addr) in peers {
                    swarm.behaviour_mut().floodsub.remove_node_from_partial_view(&peer_id);
                }
            }
            _ => {}
        }
    }
}

async fn handle_message(msg: Message, blockchain: &mut Blockchain) {
    match msg {
        Message::NewBlock(block) => {
            if blockchain.is_chain_valid() && blockchain.get_latest_block().index < block.index {
                blockchain.add_block(block.data);
            }
        }
        Message::RequestChain => {
            // Handle chain request
        }
        Message::ResponseChain(chain) => {
            if blockchain.is_chain_valid() && chain.len() > blockchain.chain.len() {
                blockchain.chain = chain;
            }
        }
    }
}

#[derive(libp2p::NetworkBehaviour)]
struct MyBehaviour {
    floodsub: Floodsub,
    mdns: Mdns,
    #[behaviour(ignore)]
    blockchain: Blockchain,
}

#[derive(Debug)]
enum MyEvent {
    Floodsub(FloodsubEvent),
    Mdns(MdnsEvent),
}

impl From<FloodsubEvent> for MyEvent {
    fn from(event: FloodsubEvent) -> Self {
        MyEvent::Floodsub(event)
    }
}

impl From<MdnsEvent> for MyEvent {
    fn from(event: MdnsEvent) -> Self {
        MyEvent::Mdns(event)
    }
}
