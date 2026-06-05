use std::net;
use std::sync::Arc;

use ed25519_dalek::Signature;
use rand::rngs::SysRng;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use serde::{Deserialize, Serialize};
use tokio;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time;

use reticulum::destination::{SingleInputDestination, SingleOutputDestination};
use reticulum::destination::link::{LinkEvent, LinkId, LinkStatus};
use reticulum::transport::Transport;
use reticulum::hash::AddressHash;

use crate::*;

pub const CONFIG_PATH: &str = "Gc.toml";
pub const LINK_AUTH_TIMEOUT: time::Duration = time::Duration::from_secs(15);

const fn default_announce_interval_seconds() -> u64 { 5 }

pub struct Gc {
  config: Config,
  radio_client: Option<SharedRadioClient>
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
  pub log_level: String,
  #[serde(default)]
  pub log_throughput: bool,
  #[serde(default)]
  pub log_mavlink: Option<u64>,
  #[serde(default)]
  pub gc_udp_address: Option<net::Ipv4Addr>,
  pub gc_udp_subnet: net::Ipv4Addr,
  pub gc_udp_port: u16,
  pub gc_reply_port: u16,
  #[serde(default="default_announce_interval_seconds")]
  pub announce_interval_seconds: u64,
  // TODO: deserialize AddressHash
  pub fc_destination: String,
  #[serde(flatten)]
  pub radio_config: RadioConfig
}

#[derive(Debug)]
pub enum Error {
  IoError(std::io::Error),
  RnsError(reticulum::error::RnsError)
}

/// Link authorization state machine.
#[derive(Debug)]
enum LinkState {
  Disconnected,
  /// Instant is the timeout for this auth
  PendingAuth(time::Instant, LinkId, Nonce),
  /// Instant is the timeout for this auth
  PendingKey(time::Instant, LinkId, Nonce, AuthPayload),
  Authenticated(LinkId)
}

type Nonce = [u8; AUTH_NONCE_BYTES];
type AuthPayload = [u8; AUTH_SIZE];

/// Mavlink heartbeat used for probing for ground station server
fn heartbeat() -> Result<Vec<u8>, mavlink::error::MessageWriteError>  {
    use std::sync::atomic::{AtomicU8, Ordering};
    use mavlink::MavHeader;
    use mavlink::common::{
      HEARTBEAT_DATA,
      MavAutopilot,
      MavMessage,
      MavType,
      MavModeFlag,
      MavState,
    };
    static SEQNUM: AtomicU8 = AtomicU8::new(0);
    let sequence = SEQNUM.load(Ordering::SeqCst).wrapping_add(1);
    SEQNUM.store(sequence, Ordering::SeqCst);
    let header = MavHeader {
        system_id: 1,     // your system ID (1 is typical for a vehicle)
        component_id: 1,  // autopilot component
        sequence
    };
    let heartbeat = MavMessage::HEARTBEAT(HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: MavType::MAV_TYPE_QUADROTOR, // or MAV_TYPE_GENERIC, etc.
        autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
        base_mode: MavModeFlag::MAV_MODE_FLAG_MANUAL_INPUT_ENABLED |
          MavModeFlag::MAV_MODE_FLAG_STABILIZE_ENABLED |
          MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
        system_status: MavState::MAV_STATE_STANDBY,
        mavlink_version: 3, // always 3 for MAVLink v2
    });
    let mut out = vec![];
    let _ = mavlink::write_v1_msg::<mavlink::common::MavMessage, _>(
      &mut out, header, &heartbeat)?;
    Ok(out)
}

impl Gc {
  pub fn new(config: Config, radio_client: Option<SharedRadioClient>) -> Self {
    Gc { config, radio_client }
  }

  pub async fn run(&self,
    transport: Transport,
    data_destination: Arc<Mutex<SingleInputDestination>>
  ) -> Result<(), Error> {
    log::info!("running gc");
    let fc_destination = AddressHash::new_from_hex_string(&self.config.fc_destination)
      .map_err(|err|{
        log::error!("error parsing fc destination hash: {err:?}");
        Error::RnsError(err)
      })?;
    log::info!("fc destination: {fc_destination}");
    let data_destination_hash = data_destination.lock().await.desc.address_hash;
    let link_state = Arc::new(Mutex::new(LinkState::Disconnected));
    let throughput = Arc::new(Mutex::new(Throughput::new_gc()));
    let mavlink_log = if let Some(max_size) = self.config.log_mavlink {
      let f = BasicRollingFileAppender::new("gc-mavlink.log",
        RollingConditionBasic::new().max_size(max_size), 1
      ).map_err(|err|{
        log::error!("error creating mavlink log: {err}");
        Error::IoError(err)
      })?;
      Some(Arc::new(Mutex::new(f)))
    } else {
      None
    };
    // ping radio client
    let ping_radio_client_loop = async || {
      loop {
        if let Some(radio_client) = self.radio_client.as_ref() {
          match radio_client.lock().await.ping().await {
            Ok(()) => log::trace!("kaonic radio client ping ok"),
            Err(err) => {
              log::error!("kaonic radio client ping error: {err:?}");
              break
            }
          }
        }
        time::sleep(time::Duration::from_secs(10)).await
      }
    };
    // send announces
    let send_announce_loop = async || loop {
      log::debug!("sending announce");
      transport.send_announce(&data_destination, None).await;
      time::sleep(time::Duration::from_secs(self.config.announce_interval_seconds))
        .await;
      { // check link state and auth timeout
        let mut link_state = link_state.lock().await;
        link_state.check_status(&transport).await;
        link_state.auth_timeout(&transport).await;
      }
    };
    // recv announces
    let recv_announce_loop = async || {
      const ANNOUNCE_RECV_TIMEOUT: time::Duration = time::Duration::from_secs(5);
      let mut announce_recv = transport.recv_announces().await;
      loop {
        match time::timeout(ANNOUNCE_RECV_TIMEOUT, announce_recv.recv()).await {
          Ok(Ok(announce)) => {
            let destination = announce.destination.lock().await;
            let address = destination.desc.address_hash;
            if address == fc_destination {
              log::debug!("got fc destination announce {address}");
              link_state.lock().await.got_fc_announce(&transport, &destination).await;
            } else {
              log::debug!("got announce: {address}");
            }
          }
          Ok(Err(err)) => {
            log::error!("error receiving announces: {err}");
            break
          }
          Err(_) => {}
        }
      }
    };
    // listen for packets from ground control station
    let ground_station_address = Arc::new(Mutex::new(None));
    log::info!("creating ground control UDP reply socket for port {}",
      self.config.gc_reply_port);
    let socket = UdpSocket::bind(format!("0.0.0.0:{}", self.config.gc_reply_port)).await
      .map_err(Error::IoError)?;
    let socket_loop = async || {
      let ground_station_timeout = time::Duration::from_secs(10);
      let mut mavlink_parser = MavlinkParser::new();
      loop {
        // use configured ground station address if present
        if let Some(addr) = self.config.gc_udp_address.as_ref() {
          let socket_addr = net::SocketAddrV4::new(*addr, self.config.gc_udp_port).into();
          log::info!("using configured ground station address: {socket_addr}");
          let bytes = match heartbeat() {
            Ok(bytes) => bytes,
            Err(err) => {
              log::error!("error creating heartbeat packet: {err}");
              return
            }
          };
          match socket.send_to(&bytes, socket_addr).await {
            Ok(_) => {}
            Err(err) => if err.kind() == std::io::ErrorKind::NetworkUnreachable {
              // network may not yet be reachable
              log::warn!("network unreachable");
              time::sleep(time::Duration::from_secs(4)).await;
              continue
            } else {
              log::error!("error sending message to ground station: {err}");
              return
            }
          }
          *ground_station_address.lock().await = Some(socket_addr);
        } else {
          // search for ground station on network at gc_udp_port
          // TODO: would be nice to use mdns here but mdns crate didn't seem to work
          let mut n = 2;  // iterate over addresses
          let mut buf = vec![0; 64];
          let mut t_search = time::Instant::now() - time::Duration::from_secs (5);
          loop {
            let subnet = self.config.gc_udp_subnet.octets();
            let now = time::Instant::now();
            if now - t_search >= time::Duration::from_secs (5) {
              log::info!("searching for ground station on subnet {}.{}.{}.0/24 at port {}",
                subnet[0], subnet[1], subnet[2], self.config.gc_udp_port);
              t_search = now;
            }
            let address = net::SocketAddrV4::new(
              net::Ipv4Addr::new(subnet[0], subnet[1], subnet[2], n), self.config.gc_udp_port);
            let bytes = match heartbeat() {
              Ok(bytes) => bytes,
              Err(err) => {
                log::error!("error creating heartbeat packet: {err}");
                return
              }
            };
            match socket.send_to(&bytes, address).await {
              Ok(_) => {}
              Err(err) => if err.kind() == std::io::ErrorKind::NetworkUnreachable {
                // network may not yet be reachable
                if t_search == now {
                  log::warn!("network unreachable");
                }
              } else {
                log::error!("error searching for ground station: {err}");
                return
              }
            }
            if let Ok(Ok((_, peer))) = tokio::time::timeout(
              time::Duration::from_millis(50), socket.recv_from(&mut buf[..])
            ).await {
              log::info!("got ground station reply from {peer}");
              *ground_station_address.lock().await = Some(peer);
              break
            }
            n += 1;
            if n == 255 {
              n = 2;
            }
          }
        }
        log::info!("listening for UDP packets on port {}",
          socket.local_addr().unwrap().port());
        let mut buf = vec![0u8; 2usize.pow(16)];
        loop {
          // read socket
          match time::timeout(ground_station_timeout, socket.recv_from(&mut buf[1..])).await {
            Ok(Ok((size, src))) => {
              if size == 0 {
                log::warn!("zero size UDP packet data");
                continue
              }
              throughput.lock().await.ground_station_bytes(size as u64);
              let data = &buf[..size+1];
              match str::from_utf8(data) {
                Ok(text) => log::trace!("received from {}: {}", src, text),
                Err(_) => log::trace!("received non-UTF8 data from {}: {:?}", src, data),
              }
              if let Some(link_id) = link_state.lock().await.authenticated_link_id() {
                if let Some(link) = transport.find_in_link(&link_id).await {
                  // if the link is active, forward to flight controller
                  let link = link.lock().await;
                  log::trace!("sending data on link ({link_id})");
                  match link.data_packet(data) {
                    Ok(packet) => {
                      drop(link); // drop to prevent deadlock
                      transport.send_packet(packet).await;
                      if self.config.log_throughput {
                        throughput.lock().await.send_packet(data.len() as u32);
                      }
                    }
                    Err(err) => log::error!("error creating data packet: {err:?}")
                  }
                } else {
                  log::error!("could not find data link ({link_id})")
                }
              }
              if let Some(mavlink_log) = mavlink_log.clone() {
                let frames = mavlink_parser.parse(data);
                log_mavlink(mavlink_log, "ground station udp", frames).await;
              }
            }
            Ok(Err(e)) => log::error!("error receiving packet: {e}"),
            Err(_) => {
              log::warn!("ground station socket no data received in \
                {ground_station_timeout:?}, assumed disconnected");
              let _ = ground_station_address.lock().await.take();
              break
            }
          }
        }
      }
    };
    // receive upstream link data
    let link_event_loop = async || {
      let _fc_destination =
        match AddressHash::new_from_hex_string(&self.config.fc_destination) {
          Ok(dest) => dest,
          Err(err) => {
            log::error!("error parsing fc destination hash: {err:?}");
            return
          }
        };
      let mut in_link_events = transport.in_link_events();
      let mut mavlink_parser = MavlinkParser::new();
      loop {
        match in_link_events.recv().await {
          Ok(link_event) => {
            log::trace!("got in link event with address hash: {}",
              link_event.address_hash);
            if link_event.address_hash == data_destination_hash {
              // data link event
              match link_event.event {
                LinkEvent::Data(payload) => {
                  log::trace!("data link {} payload ({})", link_event.id, payload.len());
                  let data = payload.as_slice();
                  if data[0] == 0x0 {
                    // data packet
                    if let Some(link_id) = link_state.lock().await
                      .authenticated_link_id()
                    {
                      if link_id != link_event.id {
                        log::warn!("received data event on non-autheticated link {}",
                          link_event.id);
                        let _ = transport.link_close(link_event.id).await.map_err(|err|
                          log::error!("error closing link {link_id}: {err:?}"));
                      }
                      if let Some(target) = ground_station_address.lock().await.clone() {
                        log::trace!("send payload ({}) to {target}", payload.len());
                        match socket.send_to(&payload.as_slice(), target).await {
                          Ok(n) => log::trace!("socket sent {n} bytes"),
                          Err(err) => {
                            log::error!("socket error sending bytes: {err:?}");
                            break
                          }
                        }
                        if self.config.log_throughput {
                          throughput.lock().await.recv_packet(payload.len() as u32);
                        }
                        if let Some(mavlink_log) = mavlink_log.clone() {
                          let frames = mavlink_parser.parse(payload.as_slice());
                          log_mavlink(mavlink_log, "flight controller link", frames).await;
                        }
                      } else {
                        log::trace!("dropping data packet: no ground station address");
                      }
                    } else {
                      log::trace!("dropping data packet: link not authenticated");
                    }
                  } else {
                    // auth packet
                    debug_assert_eq!(data[0], LINK_AUTH_BYTE);
                    let auth_data = &data[1..1 + AUTH_SIZE];
                    link_state.lock().await
                      .got_auth(&transport, link_event.id, fc_destination, auth_data)
                      .await;
                  }
                }
                LinkEvent::Activated => {
                  log::info!("data link activated {}", link_event.id);
                  link_state.lock().await.link_activated(&transport, link_event.id)
                    .await
                }
                LinkEvent::Closed => {
                  log::info!("data link closed {}", link_event.id);
                  link_state.lock().await.link_closed().await;
                }
                LinkEvent::Proof(_) => {}
              }
            }
          }
          Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            log::debug!("recv in link event lagged: {n}");
          }
          Err(err) => {
            log::error!("recv in link event error: {err:?}");
            break
          }
        }
      }
    };
    let throughput_log_loop = async || {
      if self.config.log_throughput {
        loop {
          time::sleep(time::Duration::from_secs(THROUGHPUT_LOG_FREQUENCY_SECONDS)).await;
          throughput.lock().await.log();
        }
      } else {
        std::future::pending::<()>().await;
      }
    };
    // run
    tokio::select!{
      _ = throughput_log_loop() =>
        log::info!("throughput log loop exited: shutting down"),
      _ = ping_radio_client_loop() =>
        log::info!("ping radio client loop exited: shutting down"),
      _ = send_announce_loop() => log::info!("send announce loop exited: shutting down"),
      _ = recv_announce_loop() => log::info!("recv announce loop exited: shutting down"),
      _ = socket_loop() => log::info!("socket loop exited: shutting down"),
      _ = link_event_loop() => log::info!("link event loop exited: shutting down"),
      _ = tokio::signal::ctrl_c() => log::info!("got ctrl-c: shutting down")
    }
    Ok(())
  }
}

impl LinkState {
  pub fn link_id(&self) -> Option<LinkId> {
    match self {
      LinkState::PendingAuth(_, id, _) | LinkState::PendingKey(_, id, _, _) |
      LinkState::Authenticated(id) => Some(*id),
      LinkState::Disconnected => None
    }
  }

  pub fn authenticated_link_id(&self) -> Option<LinkId> {
    match self {
      LinkState::Authenticated(id) => Some(*id),
      _ => None
    }
  }

  /// Disconnected -> PendingAuth
  pub async fn link_activated(&mut self, transport: &Transport, link_id: LinkId) {
    use rand::TryRng;
    match self {
      LinkState::PendingAuth(_, id, _) |
      LinkState::PendingKey(_, id, _, _) |
      LinkState::Authenticated(id) => {
        log::debug!("got link activated event for link {link_id} but link {id} is \
          already active");
        return
      }
      LinkState::Disconnected => {} // continue
    }
    // send nonce to request auth
    let mut nonce = [0u8; AUTH_NONCE_BYTES];
    let mut nonce_buffer = [LINK_AUTH_BYTE; AUTH_NONCE_BYTES + 1];
    let close_link = async || {
      let _ = transport.link_close(link_id).await.map_err(|err|
        log::error!("error closing link {link_id}: {err:?}"));
    };
    if let Err(err) = SysRng.try_fill_bytes(&mut nonce[..]) {
      log::error!("error filling nonce bytes: {err}");
      close_link().await;
      return
    }
    if let Some(link) = transport.find_in_link(&link_id).await.clone() {
      log::debug!("sending nonce for link {link_id}");
      nonce_buffer[1..].copy_from_slice(&nonce);
      let link = link.lock().await;
      match link.data_packet(&nonce_buffer) {
        Ok(packet) => {
          drop(link);
          transport.send_packet(packet).await;
        }
        Err(err) => {
          log::error!("error creating nonce packet, closing link: {err:?}");
          close_link().await;
          return
        }
      }
    } else {
      log::error!("error sending nonce: could not find link {link_id}");
      close_link().await;
      return
    }
    *self = LinkState::PendingAuth(
      time::Instant::now() + LINK_AUTH_TIMEOUT,
      link_id,
      nonce)
  }

  /// PendingAuth|PendingKey -> Disconnected
  pub async fn auth_timeout(&mut self, transport: &Transport) {
    match self {
      LinkState::PendingAuth(timeout, link_id, _) |
      LinkState::PendingKey(timeout, link_id, _, _) =>
        if time::Instant::now() >= *timeout {
          let _ = transport.link_close(*link_id).await.map_err(|err|
            log::error!("error closing link {link_id}: {err:?}"));
          *self = LinkState::Disconnected;
        }
      LinkState::Disconnected | LinkState::Authenticated(_) => {}
    }
  }

  pub async fn got_auth(&mut self,
    transport: &Transport,
    link_id: LinkId,
    fc_destination: AddressHash,
    auth_data: &[u8]
  ) {
    let close_link = async || {
      let _ = transport.link_close(link_id).await.map_err(|err|
        log::error!("error closing link {link_id}: {err:?}"));
    };
    // check state and link id and get sent nonce
    let (timeout, sent_nonce) = match self {
      LinkState::PendingAuth(timeout, id, nonce) => {
        if *id != link_id {
          log::warn!("waiting for auth on link {id} but got auth data on link \
            {link_id}");
          close_link().await;
        }
        (*timeout, *nonce)
      }
      state => {
        log::warn!("invalid link state for got auth event: {state:?}");
        return
      }
    };
    // check payload size
    let auth_payload = match AuthPayload::try_from (auth_data) {
      Ok(auth) => auth,
      Err(err) => {
        log::error!("auth payload not valid size: {err}");
        close_link().await;
        *self = LinkState::Disconnected;
        return
      }
    };
    // check that auth destination matches fc destination
    let auth_dest = match auth_payload[..ADDRESS_HASH_SIZE].try_into()
      .map(AddressHash::new)
    {
      Ok(dest) => dest,
      Err(err) => {
        log::error!("invalid auth destination hash: {err}");
        close_link().await;
        *self = LinkState::Disconnected;
        return
      }
    };
    if auth_dest != fc_destination {
      log::warn!("{auth_dest} is not authorized: only {fc_destination} can connect");
      close_link().await;
      *self = LinkState::Disconnected;
      return
    }
    // validate signature
    if let Some(destination) = transport.get_out_destination(&auth_dest).await {
      let destination = destination.lock().await;
      debug_assert_eq!(destination.desc.address_hash, auth_dest);
      match validate_signature(&*destination, link_id, auth_payload, sent_nonce) {
        Ok(()) => {
          *self = LinkState::Authenticated(link_id);
        }
        Err(()) => {
          close_link().await;
          *self = LinkState::Disconnected;
        }
      }
    } else {
      log::debug!("could not yet authenticate peer {auth_dest} on link {link_id}: \
        missing verify key");
      *self = LinkState::PendingKey(timeout, link_id, sent_nonce, auth_payload);
    }
  }

  pub async fn got_fc_announce(&mut self,
    transport: &Transport,
    destination: &SingleOutputDestination
  ) {
    let (link_id, sent_nonce, auth_payload) = match self {
      LinkState::PendingKey(_, id, nonce, auth) => (*id, *nonce, *auth),
      _ => return
    };
    match validate_signature(&*destination, link_id, auth_payload, sent_nonce) {
      Ok(()) => {
        *self = LinkState::Authenticated(link_id);
      }
      Err(()) => {
        let _ = transport.link_close(link_id).await.map_err(|err|
          log::error!("error closing link {link_id}: {err:?}"));
        *self = LinkState::Disconnected;
      }
    }
  }

  pub async fn check_status(&mut self, transport: &Transport) {
    if let Some(link_id) = self.link_id() {
      if let Some(link) = transport.find_in_link(&link_id).await {
        let status = link.lock().await.status();
        match status {
          // the link ID should only be set by the LinkActivated event; the link
          // should never become pending or handshake after this event
          LinkStatus::Closed |  LinkStatus::Pending | LinkStatus::Handshake => {
            log::warn!("data link is not open {status:?}, discarding link");
            *self = LinkState::Disconnected;
          }
          _ => {}
        }
      } else {
        log::error!("could not find link {link_id}");
        *self = LinkState::Disconnected;
      }
    }
  }

  pub async fn link_closed(&mut self) {
    *self = LinkState::Disconnected;
  }
}

fn validate_signature(
  destination: &SingleOutputDestination,
  link_id: LinkId,
  auth_payload: AuthPayload,
  sent_nonce: Nonce
) -> Result<(), ()> {
  let auth_dest = destination.desc.address_hash;
  let mut sig_buffer = [0u8; SIG_BUFFER_SIZE];
  match Signature::from_slice(&auth_payload[ADDRESS_HASH_SIZE..]) {
    Ok(signature) => {
      sig_buffer[..ADDRESS_HASH_SIZE]
        .copy_from_slice(&auth_payload[..ADDRESS_HASH_SIZE]);
      sig_buffer[ADDRESS_HASH_SIZE..].copy_from_slice(&sent_nonce);
      destination.identity.verifying_key.verify_strict(&sig_buffer, &signature)
        .map(|()| log::info!("authenticated fc peer {auth_dest} on link {link_id}"))
        .map_err(|err| log::error!("auth signature validation failed for {auth_dest} \
          on link {link_id}: invalid signature: {err}"))
    }
    Err(err) => {
      log::error!("failed to load signature bytes for peer {auth_dest} auth  packet: \
        {err}");
      Err(())
    }
  }
}
