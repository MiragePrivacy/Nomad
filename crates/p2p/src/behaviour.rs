use std::{
    hash::{DefaultHasher, Hash, Hasher},
    task::ready,
    time::Duration,
};

use libp2p::{
    gossipsub, identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    ping,
    swarm::{dummy, NetworkBehaviour, ToSwarm},
    Multiaddr, PeerId, StreamProtocol,
};
use nomad_types::SignalPayload;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{shutdown::Shutdown, P2pConfig, MIRAGE_DISCOVERY_ID, MIRAGE_MESHSUB_ID};

#[derive(NetworkBehaviour)]
pub struct MirageBehavior {
    pub shutdown: Shutdown,
    pub signal: SignalBehavior,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub kad: kad::Behaviour<MemoryStore>,
    pub gossipsub: gossipsub::Behaviour,
}

impl MirageBehavior {
    pub fn new(keypair: &Keypair, config: &P2pConfig) -> Self {
        let peer_id = PeerId::from_public_key(&keypair.public());

        let shutdown = Shutdown::default();
        let signal = SignalBehavior::default();
        let ping = ping::Behaviour::default();

        let identity_config = identify::Config::new("0.1.0".into(), keypair.public());
        let identify = identify::Behaviour::new(identity_config);

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .protocol_id_prefix(MIRAGE_MESHSUB_ID)
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(gossipsub::ValidationMode::None)
            .message_id_fn(|message: &gossipsub::Message| {
                let mut h = DefaultHasher::new();
                message.data.hash(&mut h);
                gossipsub::MessageId::from(h.finish().to_string())
            })
            .build()
            .expect("Failed to make the gossipsub conf");
        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Author(peer_id),
            gossipsub_config,
        )
        .unwrap();

        let mut kad_config = kad::Config::new(StreamProtocol::new(MIRAGE_DISCOVERY_ID));
        kad_config.set_periodic_bootstrap_interval(Some(config.bootstrap_interval));
        let kad_store = MemoryStore::new(peer_id);
        let mut kad = kad::Behaviour::with_config(peer_id, kad_store, kad_config);
        kad.set_mode(Some(kad::Mode::Server));

        Self {
            shutdown,
            signal,
            identify,
            ping,
            kad,
            gossipsub,
        }
    }
}

/// Simple event wrapper around the incoming signal channel
#[derive(Default)]
pub struct SignalBehavior {
    rx: Option<UnboundedReceiver<SignalPayload>>,
}

impl SignalBehavior {
    pub fn connect_rx(&mut self, rx: UnboundedReceiver<SignalPayload>) {
        self.rx = Some(rx);
    }
}

impl NetworkBehaviour for SignalBehavior {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = SignalPayload;

    fn handle_established_inbound_connection(
        &mut self,
        _: libp2p::swarm::ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: libp2p::swarm::ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: libp2p::core::Endpoint,
        _: libp2p::core::transport::PortUse,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _event: libp2p::swarm::FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: libp2p::swarm::ConnectionId,
        _: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<libp2p::swarm::ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>>
    {
        let mut res = None;
        if let Some(rx) = self.rx.as_mut() {
            res = ready!(rx.poll_recv(cx));
        }
        match res {
            Some(signal) => std::task::Poll::Ready(ToSwarm::GenerateEvent(signal)),
            None => {
                self.rx = None;
                std::task::Poll::Pending
            }
        }
    }
}
