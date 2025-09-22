use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::mpsc::SyncSender,
};

use nomad_types::SignalPayload;

pub fn main() -> eyre::Result<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel(256);
    let mut stream = TcpStream::connect("127.0.0.1:8888")?;

    // annouce sgx report on the stream

    let report = &report_for_key([0u8; 64]);
    let len = (report.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(report)?;

    // spawn read thread for processing incoming signals
    let reader = stream.try_clone()?;
    std::thread::spawn(|| read_signals(reader, tx).expect("read thread failed"));

    // process signals
    loop {
        let _signal = rx.recv()?;
        todo!()
    }
}

#[cfg(target_env = "sgx")]
fn report_for_key(data: [u8; 64]) -> Vec<u8> {
    let targetinfo = sgx_isa::Targetinfo::from(Report::for_self());
    sgx_isa::Report::for_target(&targetinfo, &data)
}
#[cfg(not(target_env = "sgx"))]
fn report_for_key(data: [u8; 64]) -> Vec<u8> {
    data.to_vec()
}

fn read_signals(mut stream: TcpStream, tx: SyncSender<SignalPayload>) -> eyre::Result<()> {
    loop {
        // Read u32 length prefixed signal payload from the stream
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;

        let len = u32::from_be_bytes(len) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;

        let signal: SignalPayload = serde_json::from_slice(&payload)?;
        tx.send(signal)?;
    }
}
