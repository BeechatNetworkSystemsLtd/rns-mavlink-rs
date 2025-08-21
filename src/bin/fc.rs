use std::str;
use std::sync::Arc;
use std::time;

use log;
use tokio::net::UdpSocket;

use reticulum::destination::{DestinationName, SingleInputDestination};
use reticulum::destination::link::{Link, LinkEvent, LinkId};
use reticulum::identity::PrivateIdentity;
use reticulum::iface::tcp_client::TcpClient;
use reticulum::transport::{Transport, TransportConfig};
use reticulum::hash::AddressHash;

pub async fn run(transport: Transport) {
  let link: Arc<tokio::sync::Mutex<Option<Arc<tokio::sync::Mutex<Link>>>>> =
    Arc::new(tokio::sync::Mutex::new(None));
  let server_destination =
    match AddressHash::new_from_hex_string("5285c511344e3b5b3a765229554e4da9") {
      Ok(dest) => dest,
      Err(err) => {
        log::error!("error parsing server destination hash: {err:?}");
        return
      }
    };
  let socket = UdpSocket::bind("0.0.0.0:14550").await.unwrap();
  // set up links
  let link_loop = async || {
    let mut announce_recv = transport.recv_announces().await;
    // TODO: continue looping after link is created?
    while let Ok(announce) = announce_recv.recv().await {
      let destination = announce.destination.lock().await;
      if destination.desc.address_hash == server_destination {
        *link.lock().await = Some(transport.link(destination.desc).await);
      }
    }
  };
  // listen to socket and forward to links
  let read_socket_loop = async || {
    loop {
      if let Some(_link) = link.lock().await.as_ref() {
        log::info!("Listening for UDP packets on port 14550...");
        let mut buf = vec![0u8; 1024];
        loop {
          match socket.recv_from(&mut buf).await {
            Ok((size, src)) => {
              let data = &buf[..size];
              match str::from_utf8(data) {
                Ok(text) => log::trace!("Received from {}: {}", src, text),
                Err(_) => log::trace!("Received non-UTF8 data from {}: {:?}", src, data),
              }
              transport.send_to_all_out_links(data).await;
            }
            Err(e) => {
              log::error!("Error receiving packet: {}", e);
            }
          }
        }
      } else {
        tokio::time::sleep(time::Duration::from_millis(100)).await
      }
    }
  };
  // forward upstream link messages to socket
  let write_socket_loop = async || {
    let mut out_link_events = transport.out_link_events();
    let target = "127.0.0.1:18570";
    while let Ok(link_event) = out_link_events.recv().await {
      match link_event.event {
        LinkEvent::Data(payload) => if link_event.address_hash == server_destination {
          log::trace!("link {} payload ({})", link_event.id, payload.len());
          match socket.send_to(payload.as_slice(), target).await {
            Ok(n) => log::trace!("socket sent {n} bytes"),
            Err(err) => {
              log::error!("socket error sending bytes: {err:?}");
              break
            }
          }
        }
        LinkEvent::Activated => if link_event.address_hash == server_destination {
          log::debug!("link activated {}", link_event.id);
        }
        LinkEvent::Closed => if link_event.address_hash == server_destination {
          log::debug!("link closed {}", link_event.id);
        }
      }
    }
  };
  // run
  tokio::select!{
    _ = read_socket_loop() => log::info!("read socket loop exited: shutting down"),
    _ = write_socket_loop() => log::info!("write socket loop exited: shutting down"),
    _ = link_loop() => log::info!("link loop exited: shutting down"),
    _ = tokio::signal::ctrl_c() => log::info!("got ctrl-c: shutting down")
  }
}

#[tokio::main]
async fn main() {
  // init logging
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("DEBUG")).init();
  // start reticulum
  let id = PrivateIdentity::new_from_name("mavlink-rns-fc");
  let transport = Transport::new(TransportConfig::new("fc", &id, true));
  let _ = transport.iface_manager().lock().await
    .spawn(TcpClient::new("192.168.1.131:4242"), TcpClient::spawn);
  let destination = SingleInputDestination::new(id, DestinationName::new("mavlink_rns", "client"));
  log::info!("created destination: {}", destination.desc.address_hash);
  run(transport).await
}
