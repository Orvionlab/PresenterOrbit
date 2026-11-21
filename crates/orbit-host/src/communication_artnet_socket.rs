use super::{timeout, DecoderFactory, Exchange, Worker, SEQUENCE};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[path = "communication_artnet_discovery.rs"]
mod discovery;

pub(super) fn run(worker: &Worker, device: &Value, factory: DecoderFactory) {
    let reconnect = device["input"]["autoReconnect"] == true;
    while worker.alive.load(Ordering::Acquire) {
        let bound = bind(device);
        match bound {
            Ok(socket) => {
                if let Ok(mut status) = worker.status.lock() {
                    status["sessionId"] =
                        json!(SEQUENCE.fetch_add(1, Ordering::Relaxed).to_string());
                    status["listening"] = json!(true);
                    status["reconnecting"] = json!(false);
                    status["error"] = Value::Null;
                    (worker.changed)(status.clone());
                }
                if let Err(error) = receive(worker, device, &socket, factory.clone()) {
                    worker.notify(Some(error), false);
                }
            }
            Err(error) => worker.notify(Some(error), false),
        }
        if let Ok(mut status) = worker.status.lock() {
            status["listening"] = json!(false);
            status["reconnecting"] = json!(reconnect && worker.alive.load(Ordering::Acquire));
            (worker.changed)(status.clone());
        }
        if !reconnect {
            break;
        }
        for _ in 0..10 {
            if !worker.alive.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}
fn bind(device: &Value) -> Result<UdpSocket, String> {
    let address = device["address"]
        .as_str()
        .ok_or("An Art-Net address is required.")?;
    let socket = UdpSocket::bind((address, 6454)).map_err(|error| error.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| error.to_string())?;
    Ok(socket)
}
fn receive(
    worker: &Worker,
    device: &Value,
    socket: &UdpSocket,
    factory: DecoderFactory,
) -> Result<(), String> {
    let mut bytes = [0; 65536];
    let mut peers = HashMap::<SocketAddr, Exchange>::new();
    let mut replies = Vec::<(Instant, SocketAddr, Vec<u8>)>::new();
    let mut counter = 0u16;
    while worker.alive.load(Ordering::Acquire) {
        let now = Instant::now();
        let mut index = 0;
        while index < replies.len() {
            if replies[index].0 <= now {
                let (_, target, packet) = replies.swap_remove(index);
                socket
                    .send_to(&packet, target)
                    .map_err(|error| error.to_string())?;
            } else {
                index += 1;
            }
        }
        match socket.recv_from(&mut bytes) {
            Ok((size, peer)) => {
                let packet = &bytes[..size];
                if size >= 10 && &packet[..8] == b"Art-Net\0" && packet[8..10] == [0, 0x20] {
                    if let Some(oem) = device["input"]["oemCode"].as_u64() {
                        let ip = local_ip(socket, peer)?;
                        let universe = device["input"]["universe"]
                            .as_u64()
                            .ok_or("An Art-Net universe is required.")?
                            as u16;
                        let title = device["title"]
                            .as_str()
                            .ok_or("An Art-Net device name is required.")?;
                        match discovery::reply(packet, ip, universe, oem as u16, title, counter) {
                            Ok(Some(reply)) => {
                                let target = SocketAddr::new(peer.ip(), 6454);
                                if let Some(index) = replies
                                    .iter()
                                    .position(|(_, address, _)| *address == target)
                                {
                                    replies.swap_remove(index);
                                }
                                if replies.len() >= 32 {
                                    worker.notify(
                                        Some("Art-Net discovery reply queue is full.".into()),
                                        true,
                                    );
                                    continue;
                                }
                                let jitter = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map_err(|error| error.to_string())?
                                    .subsec_nanos()
                                    as u64
                                    % 1001;
                                replies.push((
                                    Instant::now() + Duration::from_millis(jitter),
                                    target,
                                    reply,
                                ));
                                counter = (counter + 1) % 10000;
                                worker.notify(None, true);
                            }
                            Ok(None) => {}
                            Err(error) => worker.notify(Some(error), true),
                        }
                    }
                    continue;
                }
                if !peers.contains_key(&peer) && peers.len() >= 16 {
                    worker.notify(Some("Art-Net supports at most 16 controller sessions. Reconnect to reset controllers.".into()),true);
                    continue;
                }
                let exchange = peers.entry(peer).or_insert_with(|| (worker.exchange)());
                let mut decoder = factory();
                match decoder(packet) {
                    Ok(frames) if frames.len() == 1 => {
                        let _ = worker.exchange(exchange, &frames[0]);
                    }
                    Ok(_) => worker.notify(
                        Some("Art-Net datagrams require one complete packet.".into()),
                        true,
                    ),
                    Err(error) => worker.notify(Some(error), true),
                }
            }
            Err(error) if timeout(&error) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}
fn local_ip(socket: &UdpSocket, peer: SocketAddr) -> Result<Ipv4Addr, String> {
    let mut ip = socket.local_addr().map_err(|error| error.to_string())?.ip();
    if ip.is_unspecified() {
        let route = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
        route.connect(peer).map_err(|error| error.to_string())?;
        ip = route.local_addr().map_err(|error| error.to_string())?.ip();
    }
    match ip {
        std::net::IpAddr::V4(ip) => Ok(ip),
        _ => Err("Art-Net requires IPv4.".into()),
    }
}
