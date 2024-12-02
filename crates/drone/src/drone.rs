use crossbeam::channel::{select_biased, Receiver, Sender};
use log::{debug, error, info, trace, warn};
use rand::Rng;
use std::collections::{HashMap, HashSet};

use wg_2024::controller::{DroneCommand, NodeEvent};
use wg_2024::drone::{Drone, DroneOptions};
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{FloodRequest, FloodResponse, Nack, NackType, NodeType, Packet, PacketType};

/// Example of drone implementation
pub struct RustDrone {
    id: NodeId,
    controller_send: Sender<NodeEvent>,
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
    fn new(options: DroneOptions) -> Self {
        Self {
            id: options.id,
            controller_send: options.controller_send,
            controller_recv: options.controller_recv,
            packet_recv: options.packet_recv,
            pdr: options.pdr,
            packet_send: HashMap::new(),
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
            PacketType::Nack(_) => self.forward_packet(packet),
            PacketType::Ack(_) => self.forward_packet(packet),
            PacketType::MsgFragment(_) => self.forward_packet(packet),
            PacketType::FloodRequest(flood_request) => {
                self.handle_flood_request(flood_request, packet.session_id)
            }
            PacketType::FloodResponse(_) => self.forward_packet(packet),
        }
    }

    fn handle_command(&mut self, command: DroneCommand) -> CommandResult {
        match command {
            DroneCommand::AddSender(node_id, sender) => {
                info!("Drone '{}' connected to '{}'", self.id, node_id);
                self.packet_send.insert(node_id, sender);
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

    fn forward_packet(&self, mut packet: Packet) {
        debug!("Drone '{}' forwarding packet", self.id);
        trace!("Packet: {:?}", packet);
        if let Some(next_hop) = packet
            .routing_header
            .hops
            .get(packet.routing_header.hop_index + 1)
        {
            // list has another hop, we can try forwarding the packet
            if let Some(sender) = self.packet_send.get(next_hop) {
                // we are connected to the next hop, now we might want to drop the packet only if it's a fragment
                if rand::thread_rng().gen_range(0.0..1.0) > self.pdr
                    && matches!(packet.pack_type, PacketType::MsgFragment(_))
                {
                    // luck is on our side, we can send the packet
                    debug!("Packet has been forwarded to '{}'", next_hop);
                    packet.routing_header.hop_index += 1;
                    sender.send(packet.clone()).unwrap(); // sketchy clone
                    self.controller_send
                        .send(NodeEvent::PacketSent(packet))
                        .unwrap();
                } else {
                    // drop the packet
                    info!("Packet has been dropped from node '{}'", self.id);
                    self.return_nack(&packet, NackType::Dropped);
                    self.controller_send
                        .send(NodeEvent::PacketDropped(packet))
                        .unwrap();
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
            // malformed packet, the destination is the drone itself
            warn!("Destination is drone '{}' itself", self.id);
            self.return_nack(&packet, NackType::DestinationIsDrone);
        }
    }

    fn return_nack(&self, packet: &Packet, nack_type: NackType) {
        info!(
            "Returning NACK to sender '{}' from '{}'",
            packet.routing_header.hops[0], self.id
        );
        let mut reversed_hops = packet
            .routing_header
            .hops
            .clone()
            .split_at(packet.routing_header.hop_index)
            .0
            .to_vec();
        reversed_hops.reverse();

        let nack = Packet {
            pack_type: PacketType::Nack(Nack {
                fragment_index: if let PacketType::MsgFragment(fragment) = &packet.pack_type {
                    fragment.fragment_index
                } else {
                    0
                },
                nack_type,
            }),
            routing_header: SourceRoutingHeader {
                hops: reversed_hops,
                hop_index: 0,
            },
            session_id: packet.session_id,
        };
        self.forward_packet(nack);
    }

    fn return_flood_responce(&mut self, flood_request: FloodRequest, session_id: u64) {
        let hops = flood_request
            .path_trace
            .clone()
            .iter()
            .rev()
            .map(|(id, _)| *id)
            .collect();

        let neighbour_sender = flood_request.path_trace.last().unwrap().0;

        if let Some(sender) = self.packet_send.get(&neighbour_sender) {
            sender
                .send(Packet {
                    pack_type: PacketType::FloodResponse(FloodResponse {
                        flood_id: flood_request.flood_id,
                        path_trace: flood_request.path_trace,
                    }),
                    routing_header: SourceRoutingHeader { hops, hop_index: 0 },
                    session_id,
                })
                .unwrap();
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

        if self.passed_flood_requests.contains(&flood_request.flood_id) {
            // we have already seen this flood request
            info!(
                "Drone '{}' has already seen flood request with id '{}'",
                self.id, flood_request.flood_id
            );
            self.return_flood_responce(flood_request, session_id);
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
                    if *neighbour != flood_request.path_trace.last().unwrap().0 {
                        debug!(
                            "Drone '{}' forwarding flood request to '{}'",
                            self.id, neighbour
                        );
                        sender
                            .send(Packet {
                                pack_type: PacketType::FloodRequest(flood_request.clone()),
                                routing_header: SourceRoutingHeader {
                                    hops: Vec::new(),
                                    hop_index: 0,
                                },
                                session_id,
                            })
                            .unwrap();
                    }
                }
            } else {
                // we have only one neighbour, we can return the flood responce
                debug!(
                    "Drone '{}' has no other neighbour, returning a flood responce to {}",
                    self.id,
                    flood_request.path_trace.last().unwrap().0
                );
                self.return_flood_responce(flood_request, session_id);
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

    use wg_2024::drone::DroneOptions;
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
        let (d_controller_send, _) = unbounded();
        let (m_controller_send, d_controller_recv) = unbounded();
        let (_, t_packet_recv) = unbounded();

        let drone_options = DroneOptions {
            id: 1,
            controller_send: d_controller_send,
            controller_recv: d_controller_recv,
            packet_recv: t_packet_recv,
            pdr: 0.0,
            packet_send: HashMap::new(),
        };

        let drone_t = thread::spawn(move || {
            let mut drone = RustDrone::new(drone_options);
            drone.run();
        });

        terminate_and_assert_quit(drone_t, m_controller_send);
    }

    #[test]
    fn drone_forwards_fragment() {
        let (d_controller_send, _) = unbounded();
        let (m_controller_send, d_controller_recv) = unbounded();
        let (t_packet_send, t_packet_recv) = unbounded();
        let (neighbour_send, neighbour_recv) = unbounded();

        let drone_options = DroneOptions {
            id: 1,
            controller_send: d_controller_send,
            controller_recv: d_controller_recv,
            packet_recv: t_packet_recv,
            pdr: 0.0,
            packet_send: HashMap::new(),
        };

        let drone_t = thread::spawn(move || {
            let mut drone = RustDrone::new(drone_options);
            drone.run();
        });

        m_controller_send
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

        terminate_and_assert_quit(drone_t, m_controller_send);
    }
}
