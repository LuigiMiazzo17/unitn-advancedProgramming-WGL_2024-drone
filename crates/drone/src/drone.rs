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
