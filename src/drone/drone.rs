use crossbeam::channel::{select_biased, Receiver, Sender};
use log::{debug, error, info, trace, warn};
use rand::Rng;
use std::collections::{HashMap, HashSet};

use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::drone::Drone;
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{FloodRequest, FloodResponse, Nack, NackType, NodeType, Packet, PacketType};

/// Example of drone implementation
pub struct RustDrone {
    id: NodeId,
    controller_send: Sender<DroneEvent>,
    controller_recv: Receiver<DroneCommand>,
    packet_recv: Receiver<Packet>,
    pdr: f32,
    packet_send: HashMap<NodeId, Sender<Packet>>,
    passed_flood_requests: HashSet<u64>,
}

enum CommandResult {
    Ok,
    Quit,
}

impl Drone for RustDrone {
    fn new(
        id: NodeId,
        controller_send: Sender<DroneEvent>,
        controller_recv: Receiver<DroneCommand>,
        packet_recv: Receiver<Packet>,
        packet_send: HashMap<NodeId, Sender<Packet>>,
        pdr: f32,
    ) -> Self {
        Self {
            id,
            controller_send,
            controller_recv,
            packet_recv,
            pdr,
            packet_send,
            passed_flood_requests: HashSet::new(),
        }
    }

    fn run(&mut self) {
        loop {
            select_biased! {
                recv(self.controller_recv) -> command => {
                    if let Ok(command) = command {
                        match self.handle_command(command) {
                            CommandResult::Quit => break,
                            CommandResult::Ok => {}
                        }
                    }
                }
                recv(self.packet_recv) -> packet => {
                    if let Ok(packet) = packet {
                        self.handle_packet(packet);
                    }
                },
            }
        }
        trace!("Drone '{}' has succesfully stopped", self.id);
    }
}

impl RustDrone {
    fn handle_packet(&mut self, packet: Packet) {
        match packet.pack_type {
            PacketType::Nack(_) => self.route_packet(packet),
            PacketType::Ack(_) => self.route_packet(packet),
            PacketType::MsgFragment(_) => self.route_packet(packet),
            PacketType::FloodRequest(flood_request) => {
                self.handle_flood_request(flood_request, packet.session_id)
            }
            PacketType::FloodResponse(_) => self.route_packet(packet),
        }
    }

    fn handle_command(&mut self, command: DroneCommand) -> CommandResult {
        match command {
            DroneCommand::AddSender(node_id, sender) => {
                info!("Drone '{}' connected to '{}'", self.id, node_id);
                self.packet_send.insert(node_id, sender);
                CommandResult::Ok
            }
            DroneCommand::RemoveSender(node_id) => {
                info!("Drone '{}' disconnected from '{}'", self.id, node_id);
                if self.packet_send.remove(&node_id).is_none() {
                    warn!(
                        "Drone '{}' tried to disconnect from '{}', but it was not connected",
                        self.id, node_id
                    );
                }
                CommandResult::Ok
            }
            DroneCommand::SetPacketDropRate(pdr) => {
                info!("Drone '{}' set PDR to {}", self.id, pdr);
                self.pdr = pdr;
                CommandResult::Ok
            }
            DroneCommand::Crash => {
                info!("Drone '{}' recived crash", self.id);
                CommandResult::Quit
            }
        }
    }

    fn route_packet(&self, mut packet: Packet) {
        debug!("Drone '{}' forwarding packet", self.id);
        trace!("Packet: {:?}", packet);

        // check if the packet has another hop
        if let Some(next_hop) = packet
            .routing_header
            .hops
            .get(packet.routing_header.hop_index + 1)
        {
            // list has another hop, we can try forwarding the packet
            if let Some(sender) = self.packet_send.get(next_hop) {
                // we are connected to the next hop, now we might want to drop the packet only if it's a fragment
                if !matches!(packet.pack_type, PacketType::MsgFragment(_))
                    || rand::thread_rng().gen_range(0.0..1.0) >= self.pdr
                {
                    // luck is on our side, we can forward the packet
                    debug!("Drone '{}' forwarding packet to '{}'", self.id, next_hop);
                    packet.routing_header.hop_index += 1;

                    if let Err(e) = sender.send(packet.clone()) {
                        error!("Error sending packet: {}", e);
                        if let Err(e) = self.controller_send.send(DroneEvent::PacketDropped(packet))
                        {
                            error!("Error sending PacketDropped event: {}", e);
                        }
                    } else if let Err(e) = self.controller_send.send(DroneEvent::PacketSent(packet))
                    {
                        error!("Error sending PacketSent event: {}", e);
                    }
                } else {
                    // drop the packet
                    info!("Packet has been dropped from node '{}'", self.id);
                    self.return_nack(&packet, NackType::Dropped);
                    if let Err(e) = self.controller_send.send(DroneEvent::PacketDropped(packet)) {
                        error!("Error sending PacketDropped event: {}", e);
                    }
                }
            } else {
                // next hop is not in the list of connected nodes
                warn!(
                    "Next hop is not in the list of connected nodes for drone '{}'",
                    self.id
                );
                self.return_nack(&packet, NackType::ErrorInRouting(*next_hop));
            }
        } else {
            // the destination is the drone itself
            if !matches!(&packet.pack_type, PacketType::Nack(_)) {
                warn!("Destination is drone '{}' itself", self.id);
                self.return_nack(&packet, NackType::DestinationIsDrone);
            } else {
                debug!(
                    "Packet is a Nack, destination is drone '{}' itself",
                    self.id
                );
            }
        }
    }

    fn return_nack(&self, packet: &Packet, nack_type: NackType) {
        info!(
            "Returning NACK to sender '{}' from '{}' with reason '{:?}'",
            packet.routing_header.hops[0], self.id, nack_type
        );

        let hops = packet
            .routing_header
            .hops
            .clone()
            .split_at(packet.routing_header.hop_index + 1)
            .0
            .iter()
            .rev()
            .cloned()
            .collect();

        let nack = Packet {
            pack_type: PacketType::Nack(Nack {
                fragment_index: if let PacketType::MsgFragment(fragment) = &packet.pack_type {
                    fragment.fragment_index
                } else {
                    0
                },
                nack_type,
            }),
            routing_header: SourceRoutingHeader { hops, hop_index: 1 },
            session_id: packet.session_id,
        };

        match self.packet_send.get(&nack.routing_header.hops[1]) {
            Some(sender) => {
                if let Err(e) = sender.send(nack.clone()) {
                    error!("Error sending NACK: {}", e);
                }
            }
            None => {
                error!(
                    "Next hop is not in the list of connected nodes for drone '{}', even though it was received from it",
                    self.id
                );
            }
        }
    }

    fn return_flood_response(
        &mut self,
        flood_request: FloodRequest,
        neighbour: NodeId,
        session_id: u64,
    ) {
        let hops = flood_request
            .path_trace
            .clone()
            .iter()
            .rev()
            .map(|(id, _)| *id)
            .collect();

        if let Some(sender) = self.packet_send.get(&neighbour) {
            if let Err(e) = sender.send(Packet {
                pack_type: PacketType::FloodResponse(FloodResponse {
                    flood_id: flood_request.flood_id,
                    path_trace: flood_request.path_trace,
                }),
                routing_header: SourceRoutingHeader { hops, hop_index: 1 },
                session_id,
            }) {
                error!("Error sending flood response: {}", e);
            }
        } else {
            error!(
                    "Next hop is not in the list of connected nodes for drone '{}', even though it was received from it",
                    self.id
                );
        }
    }

    fn handle_flood_request(&mut self, mut flood_request: FloodRequest, session_id: u64) {
        trace!(
            "Drone '{}' handling flood request with id '{}'",
            self.id,
            flood_request.flood_id
        );
        flood_request.path_trace.push((self.id, NodeType::Drone));

        let sender_id = match flood_request.path_trace.last() {
            Some(a) => a.0,
            None => {
                error!("Path trace is empty");
                return;
            }
        };

        if self.passed_flood_requests.contains(&flood_request.flood_id) {
            // we have already seen this flood request
            info!(
                "Drone '{}' has already seen flood request with id '{}'",
                self.id, flood_request.flood_id
            );
            self.return_flood_response(flood_request, sender_id, session_id);
        } else {
            // never seen this flood request
            info!(
                "Drone '{}' has never seen flood request with id '{}'",
                self.id, flood_request.flood_id
            );
            self.passed_flood_requests.insert(flood_request.flood_id);

            if self.packet_send.len() > 1 {
                // we have more than one neighbour, we need to forward the flood request to all but one
                trace!(
                    "Drone '{}' has more than one neighbour, forwarding flood request",
                    self.id
                );

                for (neighbour, sender) in self.packet_send.iter() {
                    if *neighbour != sender_id {
                        debug!(
                            "Drone '{}' forwarding flood request to '{}'",
                            self.id, neighbour
                        );
                        if let Err(e) = sender.send(Packet {
                            pack_type: PacketType::FloodRequest(flood_request.clone()),
                            routing_header: SourceRoutingHeader {
                                hops: Vec::new(),
                                hop_index: 0,
                            },
                            session_id,
                        }) {
                            error!("Error sending flood request: {}", e);
                        }
                    }
                }
            } else {
                // we have only one neighbour, we can return the flood responce
                debug!(
                    "Drone '{}' has no other neighbour, returning a flood responce to {}",
                    self.id, sender_id
                );
                self.return_flood_response(flood_request, sender_id, session_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam::channel::unbounded;
    use std::thread;
    use std::time::{Duration, Instant};
    use wg_2024::packet::Ack;
    use wg_2024::tests::*;

    use wg_2024::packet::Fragment;

    fn terminate_and_assert_quit(
        drone_t: thread::JoinHandle<()>,
        controller_send: Sender<DroneCommand>,
    ) {
        assert!(!drone_t.is_finished());

        controller_send.send(DroneCommand::Crash).unwrap();

        let timeout = Duration::from_secs(1);
        let start_time = Instant::now();

        // Wait for the drone to finish
        while start_time.elapsed() < timeout {
            if drone_t.is_finished() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("Drone did not quit in time");
    }

    fn generate_random_payload() -> (u8, [u8; 128]) {
        let payload_len = rand::thread_rng().gen_range(1..128) as u8;
        let mut payload: [u8; 128] = [0; 128];
        let payload_vec = vec![rand::thread_rng().gen::<u8>(); payload_len as usize];

        for (i, byte) in payload_vec.iter().enumerate() {
            payload[i] = *byte;
        }

        (payload_len, payload)
    }

    #[test]
    fn drone_crashes_upon_cmd() {
        let (m_controller_send, d_controller_recv) = unbounded();
        let (_, t_packet_recv) = unbounded();

        let drone_t = thread::spawn(move || {
            let mut drone = RustDrone::new(
                1,
                unbounded().0,
                d_controller_recv,
                t_packet_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        });

        terminate_and_assert_quit(drone_t, m_controller_send);
    }

    #[test]
    fn drone_forwards_fragment() {
        let (update_send, update_recv) = unbounded();
        let (t_packet_send, t_packet_recv) = unbounded();
        let (neighbour_send, neighbour_recv) = unbounded();

        let drone_t = thread::spawn(move || {
            let mut drone = RustDrone::new(
                1,
                unbounded().0,
                update_recv,
                t_packet_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        });

        update_send
            .send(DroneCommand::AddSender(2, neighbour_send))
            .unwrap();

        // Wait for the drone to connect to the neighbour
        thread::sleep(Duration::from_millis(100));

        let (payload_len, payload) = generate_random_payload();
        let session_id = rand::thread_rng().gen::<u64>();

        let sending_packet = Packet {
            pack_type: PacketType::MsgFragment(Fragment {
                fragment_index: 0,
                total_n_fragments: 1,
                length: payload_len,
                data: payload,
            }),
            routing_header: SourceRoutingHeader {
                hops: vec![1, 2],
                hop_index: 0,
            },
            session_id,
        };

        // Send the packet to the drone
        t_packet_send.send(sending_packet.clone()).unwrap();

        let received_packet = neighbour_recv.recv().unwrap();

        let mut expected_packet = sending_packet;
        // The packet should have been forwarded to the neighbour
        expected_packet.routing_header.hop_index = 1;

        assert_eq!(received_packet, expected_packet);

        terminate_and_assert_quit(drone_t, update_send);
    }

    #[test]
    fn wrap_generic_fragment_forward() {
        generic_fragment_forward::<RustDrone>();
    }

    #[test]
    fn wrap_generic_fragment_drop() {
        generic_fragment_drop::<RustDrone>();
    }

    #[test]
    fn generic_chain_fragment_drop() {
        // Client 1 channels
        let (c_send, c_recv) = unbounded();
        // Server 21 channels
        let (s_send, _s_recv) = unbounded();
        // Drone 11
        let (d_send, d_recv) = unbounded();
        // Drone 12
        let (d12_send, d12_recv) = unbounded();
        // SC - needed to not make the drone crash
        let (d_command_send, d_command_recv) = unbounded();
        let (d2_command_send, d2_command_recv) = unbounded();

        // Drone 11
        let mut drone = RustDrone::new(
            11,
            unbounded().0,
            d_command_recv,
            d_recv,
            HashMap::new(),
            0.0,
        );
        // Drone 12
        let mut drone2 = RustDrone::new(
            12,
            unbounded().0,
            d2_command_recv,
            d12_recv,
            HashMap::new(),
            1.0,
        );

        d_command_send
            .send(DroneCommand::AddSender(1, c_send.clone()))
            .unwrap();

        d_command_send
            .send(DroneCommand::AddSender(12, d12_send.clone()))
            .unwrap();

        d2_command_send
            .send(DroneCommand::AddSender(11, d_send.clone()))
            .unwrap();

        d2_command_send
            .send(DroneCommand::AddSender(21, s_send.clone()))
            .unwrap();

        // Spawn the drone's run method in a separate thread
        let d_t = thread::Builder::new()
            .name("drone".to_string())
            .spawn(move || {
                drone.run();
            })
            .unwrap();

        let d2_t = thread::Builder::new()
            .name("drone2".to_string())
            .spawn(move || {
                drone2.run();
            })
            .unwrap();

        let (payload_size, payload) = generate_random_payload();

        let msg = Packet {
            pack_type: PacketType::MsgFragment(Fragment {
                fragment_index: 1,
                total_n_fragments: 1,
                length: payload_size,
                data: payload,
            }),
            routing_header: SourceRoutingHeader {
                hops: vec![1, 11, 12, 21],
                hop_index: 1,
            },
            session_id: 1,
        };

        // "Client" sends packet to the drone
        d_send.send(msg.clone()).unwrap();

        // Client receive an NACK originated from 'd2'
        assert_eq!(
            c_recv.recv().unwrap(),
            Packet {
                pack_type: PacketType::Nack(Nack {
                    fragment_index: 1,
                    nack_type: NackType::Dropped,
                }),
                routing_header: SourceRoutingHeader {
                    hop_index: 2,
                    hops: vec![12, 11, 1],
                },
                session_id: 1,
            }
        );

        terminate_and_assert_quit(d_t, d_command_send);
        terminate_and_assert_quit(d2_t, d2_command_send);
    }

    #[test]
    fn test_round_trip_message() {
        // Client<1> channels
        let (c_send, c_recv) = unbounded();
        // Server<21> channels
        let (s_send, s_recv) = unbounded();
        // Drone 11
        let (d_send, d_recv) = unbounded();
        // Drone 12
        let (d12_send, d12_recv) = unbounded();
        // SC for d11
        let (d_command_send, d_command_recv) = unbounded();
        // SC for d12
        let (d2_command_send, d2_command_recv) = unbounded();

        // Drone 11
        let mut drone = RustDrone::new(
            11,
            unbounded().0,
            d_command_recv,
            d_recv,
            HashMap::new(),
            0.0,
        );
        // Drone 12
        let mut drone2 = RustDrone::new(
            12,
            unbounded().0,
            d2_command_recv,
            d12_recv,
            HashMap::new(),
            0.0,
        );

        d_command_send
            .send(DroneCommand::AddSender(1, c_send.clone()))
            .unwrap();

        d_command_send
            .send(DroneCommand::AddSender(12, d12_send.clone()))
            .unwrap();

        d2_command_send
            .send(DroneCommand::AddSender(21, s_send.clone()))
            .unwrap();

        d2_command_send
            .send(DroneCommand::AddSender(11, d_send.clone()))
            .unwrap();

        // Spawn the drone's run method in a separate thread
        let d_t = thread::Builder::new()
            .name("drone1".to_string())
            .spawn(move || {
                drone.run();
            })
            .unwrap();

        let d2_t = thread::Builder::new()
            .name("drone2".to_string())
            .spawn(move || {
                drone2.run();
            })
            .unwrap();

        let (payload_size, payload) = generate_random_payload();

        let mut msg = Packet {
            pack_type: PacketType::MsgFragment(Fragment {
                fragment_index: 0,
                total_n_fragments: 1,
                length: payload_size,
                data: payload,
            }),
            routing_header: SourceRoutingHeader {
                hops: vec![1, 11, 12, 21],
                hop_index: 1,
            },
            session_id: rand::thread_rng().gen::<u64>(),
        };

        // "Client" sends packet to the drone
        d_send.send(msg.clone()).unwrap();

        msg.routing_header.hop_index = 3;
        // Server receives the fragment
        assert_eq!(s_recv.recv().unwrap(), msg);

        let mut ack = Packet {
            pack_type: PacketType::Ack(Ack { fragment_index: 0 }),
            routing_header: SourceRoutingHeader {
                hops: vec![21, 12, 11, 1],
                hop_index: 1,
            },
            session_id: msg.session_id,
        };

        // Server sends ack to the drone
        d12_send.send(ack.clone()).unwrap();

        ack.routing_header.hop_index = 3;
        // Client receives the ack
        assert_eq!(c_recv.recv().unwrap(), ack);

        terminate_and_assert_quit(d_t, d_command_send);
        terminate_and_assert_quit(d2_t, d2_command_send);
    }
}
