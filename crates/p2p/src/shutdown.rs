use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll,
};

use futures::task::AtomicWaker;
use libp2p::swarm::{dummy, NetworkBehaviour, ToSwarm};

#[derive(Clone, Default)]
pub struct Shutdown {
    flag: Arc<AtomicBool>,
    waker: Arc<AtomicWaker>,
}

impl Shutdown {
    pub fn shutdown(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Relaxed);
        self.waker.wake();
    }
}

impl Future for Shutdown {
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.waker.register(cx.waker());
        if self.flag.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl NetworkBehaviour for Shutdown {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = ();

    fn handle_established_inbound_connection(
        &mut self,
        _: libp2p::swarm::ConnectionId,
        _: libp2p::PeerId,
        _: &libp2p::Multiaddr,
        _: &libp2p::Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: libp2p::swarm::ConnectionId,
        _: libp2p::PeerId,
        _: &libp2p::Multiaddr,
        _: libp2p::core::Endpoint,
        _: libp2p::core::transport::PortUse,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _event: libp2p::swarm::FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: libp2p::PeerId,
        _: libp2p::swarm::ConnectionId,
        _: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<libp2p::swarm::ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>>
    {
        Future::poll(Pin::new(self), cx).map(ToSwarm::GenerateEvent)
    }
}
