use std::str;
use std::sync::Arc;
use std::time;

use log;
use tokio::net::UdpSocket;

use reticulum::destination::{DestinationName, SingleInputDestination};
use reticulum::destination::link::{Link, LinkEvent, LinkId};
use reticulum::identity::PrivateIdentity;
use reticulum::iface::tcp_server::TcpServer;
use reticulum::iface::udp::UdpInterface;
use reticulum::transport::{Transport, TransportConfig};
use reticulum::hash::AddressHash;

pub async fn run(mut transport: Transport, id: PrivateIdentity) {
  let in_destination = transport
    .add_destination(id, DestinationName::new("mavlink_rns", "server")).await;
  let in_destination_hash = in_destination.lock().await.desc.address_hash;
  log::info!("created in destination: {}", in_destination_hash);
  // send announces
  let announce_loop = async || loop {
    transport.send_announce(&in_destination, None).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
  };
  let link_id: Arc<tokio::sync::Mutex<Option<LinkId>>> = Arc::new(tokio::sync::Mutex::new(None));
  let socket = UdpSocket::bind("0.0.0.0:9999").await.unwrap();
  // socket loop
  let socket_loop = async || {
    log::info!("Listening for UDP packets on port 9999...");
    let mut buf = vec![0u8; 1024];
    loop {
      match socket.recv_from(&mut buf).await {
        Ok((size, src)) => {
          let data = &buf[..size];
          match str::from_utf8(data) {
            Ok(text) => log::trace!("Received from {}: {}", src, text),
            Err(_) => log::trace!("Received non-UTF8 data from {}: {:?}", src, data),
          }
          let link_id = link_id.lock().await;
          if let Some(link_id) = link_id.as_ref() {
            log::trace!("sending on link ({})", link_id);
            if let Some(link) = transport.find_in_link(link_id).await {
              let link = link.lock().await;
              match link.data_packet(data) {
                Ok(packet) => {
                  drop(link); // drop to prevent deadlock
                  transport.send_packet(packet).await;
                }
                Err(err) => log::error!("error creating packet: {err:?}")
              }
            } else {
              log::error!("could not find in link ({link_id})")
            }
          }
        }
        Err(e) => {
          log::error!("Error receiving packet: {}", e);
        }
      }
    }
  };
  // upstream link data
  let link_loop = async || {
    let _client_destination =
      match AddressHash::new_from_hex_string("2a2ec986877560e660b9b7d401f41531") {
        Ok(dest) => dest,
        Err(err) => {
          log::error!("error parsing client destination hash: {err:?}");
          return
        }
      };
    let mut in_link_events = transport.in_link_events();
    let target = "127.0.0.1:14550";
    loop {
      match in_link_events.recv().await {
        Ok(link_event) => match link_event.event {
          LinkEvent::Data(payload) => if link_event.address_hash == in_destination_hash {
            log::trace!("link {} payload ({})", link_event.id, payload.len());
            /*FIXME:debug*/ //log::warn!("GOT LINK DATA");
            match socket.send_to(payload.as_slice(), target).await {
              Ok(n) => log::trace!("socket sent {n} bytes"),
              Err(err) => {
                log::error!("socket error sending bytes: {err:?}");
                break
              }
            }
          }
          LinkEvent::Activated => if link_event.address_hash == in_destination_hash {
            log::info!("link activated {}", link_event.id);
            let mut link_id = link_id.lock().await;
            *link_id = Some(link_event.id);
          }
          LinkEvent::Closed => if link_event.address_hash == in_destination_hash {
            log::warn!("link closed {}", link_event.id);
            let _ = link_id.lock().await.take();
          }
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
          log::warn!("link lagged: {n}");
        }
        Err(err) => {
          log::error!("link error: {err:?}");
          break
        }
      }
    }
  };
  tokio::select!{
    _ = announce_loop() => log::info!("announce loop exited: shutting down"),
    _ = socket_loop() => log::info!("tun loop exited: shutting down"),
    _ = link_loop() => log::info!("link loop exited: shutting down"),
    _ = tokio::signal::ctrl_c() => log::info!("got ctrl-c: shutting down")
  }
}

#[tokio::main]
async fn main() {
  // init logging
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("DEBUG")).init();
  // start reticulum
  log::info!("starting reticulum");
  let id = PrivateIdentity::new_from_name("mavlink-rns-server");
  let transport = Transport::new(TransportConfig::new("server", &id, true));
  let _ = transport.iface_manager().lock().await.spawn(
    //UdpInterface::new("0.0.0.0:4242", Some("192.168.1.135:4243")),
    UdpInterface::new("0.0.0.0:4242", Some("127.0.0.1:4243")),
    UdpInterface::spawn,
    //TcpServer::new(format!("0.0.0.0:{}", 4242), transport.iface_manager()),
    //TcpServer::spawn,
  );
  run(transport, id).await
}
