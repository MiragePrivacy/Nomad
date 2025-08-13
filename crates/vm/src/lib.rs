use affair::{DedicatedThread, Executor, Socket, Worker};

/// Type alias for the worker socket
pub type VmSocket = Socket<<NomadVm as Worker>::Request, <NomadVm as Worker>::Response>;

/// A simple VM for executing signal puzzles.
/// Can be spawned as a worker using the helper method
/// [`spawn_vm_thread`] or using [`affair`] directly.
#[derive(Default)]
pub struct NomadVm {}

impl NomadVm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a new dedicated thread to run the vm worker on
    pub fn spawn(self) -> VmSocket {
        DedicatedThread::spawn(self)
    }

    /// Execute a program, returning the resulting secret material.
    /// VM state is cleared after every execution.
    pub fn execute(&mut self, _program: Vec<u8>) -> [u8; 32] {
        // TODO: implement the vm!
        [0; 32]
    }
}

impl Worker for NomadVm {
    type Request = Vec<u8>;
    type Response = [u8; 32];
    fn handle(&mut self, req: Self::Request) -> Self::Response {
        self.execute(req)
    }
}
