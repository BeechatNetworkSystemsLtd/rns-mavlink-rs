use std::sync::Arc;

use serde::Deserialize;
use tokio;
use tokio::net::UdpSocket;
use tokio::time::Duration;

use reticulum::destination::DestinationName;
use reticulum::destination::link::{Link, LinkEvent, LinkId};
use reticulum::identity::PrivateIdentity;
use reticulum::transport::Transport;
use reticulum::hash::AddressHash;

pub struct Gc {
  config: GcConfig
}

pub struct Fc {
  config: FcConfig
}

#[derive(Deserialize)]
pub struct GcConfig {
  pub log_level: String,
  pub qgc_udp_address: std::net::SocketAddr,
  pub qgc_reply_port: u16,
  // TODO: deserialize AddressHash
  pub fc_destination: String
}

#[derive(Deserialize)]
pub struct FcConfig {
  pub log_level: String,
  pub serial_port: String,
  pub serial_baud: u32,
  // TODO: deserialize AddressHash
  pub gc_destination: String
}

#[derive(Debug)]
pub enum GcError {
  IoError(std::io::Error)
}

#[derive(Debug)]
pub enum FcError {
  SerialDeviceError(tokio_serial::Error),
  RnsError(reticulum::error::RnsError)
}

impl Gc {
  pub fn new(config: GcConfig) -> Self {
    Gc { config }
  }

  pub async fn run(&self, mut transport: Transport, id: PrivateIdentity)
    -> Result<(), GcError>
  {
    let in_destination = transport
      .add_destination(id, DestinationName::new("rns_mavlink", "gc")).await;
    let in_destination_hash = in_destination.lock().await.desc.address_hash;
    log::info!("created in destination: {}", in_destination_hash);
    // send announces
    let announce_loop = async || loop {
      transport.send_announce(&in_destination, None).await;
      tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let link_id: Arc<tokio::sync::Mutex<Option<LinkId>>> =
      Arc::new(tokio::sync::Mutex::new(None));
    let socket = UdpSocket::bind(format!("0.0.0.0:{}", self.config.qgc_reply_port))
      .await.map_err(GcError::IoError)?;
    // socket loop
    let socket_loop = async || {
      log::info!("listening for UDP packets on port {}...", self.config.qgc_reply_port);
      let mut buf = vec![0u8; 1024];
      loop {
        match socket.recv_from(&mut buf).await {
          Ok((size, src)) => {
            let data = &buf[..size];
            match str::from_utf8(data) {
              Ok(text) => log::trace!("received from {}: {}", src, text),
              Err(_) => log::trace!("received non-UTF8 data from {}: {:?}", src, data),
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
          Err(e) => log::error!("error receiving packet: {}", e)
        }
      }
    };
    // upstream link data
    let link_loop = async || {
      let _fc_destination =
        match AddressHash::new_from_hex_string(&self.config.fc_destination) {
          Ok(dest) => dest,
          Err(err) => {
            log::error!("error parsing fc destination hash: {err:?}");
            return
          }
        };
      let mut in_link_events = transport.in_link_events();
      let target = self.config.qgc_udp_address;
      loop {
        match in_link_events.recv().await {
          Ok(link_event) => match link_event.event {
            LinkEvent::Data(payload) =>
              if link_event.address_hash == in_destination_hash {
                log::trace!("link {} payload ({})", link_event.id, payload.len());
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
    Ok(())
  }
}

impl Fc {
  pub fn new(config: FcConfig) -> Result<Self, ()> {
    let fc = Fc { config };
    Ok(fc)
  }

  pub async fn run(&self, transport: Transport) -> Result<(), FcError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_serial::SerialPortBuilderExt;
    let link: Arc<tokio::sync::Mutex<Option<Arc<tokio::sync::Mutex<Link>>>>> =
      Arc::new(tokio::sync::Mutex::new(None));
    let gc_destination = AddressHash::new_from_hex_string(&self.config.gc_destination)
      .map_err(|err|{
        log::error!("error parsing ground control destination hash: {err:?}");
        FcError::RnsError(err)
      })?;
    let port = tokio_serial::new(&self.config.serial_port, self.config.serial_baud)
      .open_native_async()
      .map_err(FcError::SerialDeviceError)?;
    let (mut port_reader, mut port_writer) = tokio::io::split(port);
    // set up links
    let link_loop = async || {
      let mut announce_recv = transport.recv_announces().await;
      // TODO: continue looping after link is created?
      while let Ok(announce) = announce_recv.recv().await {
        let destination = announce.destination.lock().await;
        if destination.desc.address_hash == gc_destination {
          *link.lock().await = Some(transport.link(destination.desc).await);
        }
      }
    };
    // read serial port and forward to links
    let mut read_port_loop = async || {
      loop {
        if let Some(_link) = link.lock().await.as_ref() {
          log::info!("reading from serial port {}...", self.config.serial_port);
          let mut buf = vec![0u8; 2usize.pow(16)];
          loop {
            match port_reader.read(&mut buf).await {
              Ok(n) => {
                log::trace!("read {n} bytes");

                for data in buf[..n].chunks(reticulum::packet::PACKET_MDU / 2) {
                  transport.send_to_all_out_links(data).await;
                }
              }
              Err(e) => log::error!("error reading serial port: {}", e)
            }
          }
        }
      }
    };
    // forward upstream link messages to serial port
    let mut write_port_loop = async || {
      let mut out_link_events = transport.out_link_events();
      while let Ok(link_event) = out_link_events.recv().await {
        match link_event.event {
          LinkEvent::Data(payload) => if link_event.address_hash == gc_destination {
            log::trace!("link {} payload ({})", link_event.id, payload.len());
            match port_writer.write_all(payload.as_slice()).await {
              Ok(()) => log::trace!("port sent {} bytes", payload.len()),
              Err(err) => {
                log::error!("port error sending bytes: {err:?}");
                break
              }
            }
          }
          LinkEvent::Activated => if link_event.address_hash == gc_destination {
            log::info!("link activated {}", link_event.id);
          }
          LinkEvent::Closed => if link_event.address_hash == gc_destination {
            log::warn!("link closed {}", link_event.id);
            let _ = link.lock().await.take();
          }
        }
      }
    };
    // run
    tokio::select!{
      _ = read_port_loop() => log::info!("read port loop exited: shutting down"),
      _ = write_port_loop() => log::info!("write port loop exited: shutting down"),
      _ = link_loop() => log::info!("link loop exited: shutting down"),
      _ = tokio::signal::ctrl_c() => log::info!("got ctrl-c: shutting down")
    }
    Ok(())
  }
}
