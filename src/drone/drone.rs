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
        if let Some(current_hop) = Self::get_current_hop(&packet) {
            if current_hop != self.id {
                // we received a packet with wrong current hop
                warn!(
                    "Drone '{}' received packet with wrong current hop '{}'",
                    self.id, current_hop
                );
                trace!("Packet: {:?}", packet);
            } else {
                // handle correctly the packet
                debug!("Drone '{}' processing packet", self.id);
                trace!("Packet: {:?}", packet);

                match packet.pack_type {
                    PacketType::FloodRequest(_) => self.handle_flood_request(packet),
                    _ => self.route_packet(packet),
                }
            }
        } else {
            // we received a packet with no current hop
            error!("Recived packet with no current hop");
            trace!("Packet: {:?}", packet);

            self.return_nack(&packet, NackType::UnexpectedRecipient(self.id))
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

    fn get_previous_hop(packet: &Packet) -> Option<NodeId> {
        packet
            .routing_header
            .hops
            .get(packet.routing_header.hop_index - 1)
            .cloned()
    }

    fn get_current_hop(packet: &Packet) -> Option<NodeId> {
        packet
            .routing_header
            .hops
            .get(packet.routing_header.hop_index)
            .cloned()
    }

    fn get_next_hop(packet: &Packet) -> Option<NodeId> {
        packet
            .routing_header
            .hops
            .get(packet.routing_header.hop_index + 1)
            .cloned()
    }

    fn get_source(packet: &Packet) -> Option<NodeId> {
        packet.routing_header.hops.first().cloned()
    }

    // fn get_destination(packet: &Packet) -> Option<NodeId> {
    //     packet.routing_header.hops.last().cloned()
    // }

    fn route_packet(&self, mut packet: Packet) {
        // check if the packet has another hop
        let next_hop = match Self::get_next_hop(&packet) {
            Some(next_hop) => next_hop,
            None => {
                // the destination is the drone itself
                if !matches!(&packet.pack_type, PacketType::Nack(_)) {
                    warn!("Destination is drone '{}' itself", self.id);
                    self.return_nack(&packet, NackType::DestinationIsDrone);
                } else {
                    debug!(
                        "Packet is a Nack, destination is drone '{}' itself",
                        self.id
                    );
                };
                return;
            }
        };

        // check if the next hop is in the list of connected nodes
        let forward_channel = match self.packet_send.get(&next_hop) {
            Some(sender) => sender,
            None => {
                // next hop is not in the list of connected nodes
                warn!(
                    "Next hop is not in the list of connected nodes for drone '{}'",
                    self.id
                );
                self.return_nack(&packet, NackType::ErrorInRouting(next_hop));
                return;
            }
        };

        // we are connected to the next hop, now we might want to drop the packet only if it's a fragment
        if !matches!(packet.pack_type, PacketType::MsgFragment(_))
            || rand::thread_rng().gen_range(0.0..1.0) >= self.pdr
        {
            // luck is on our side, we can forward the packet
            debug!("Drone '{}' forwarding packet to '{}'", self.id, next_hop);
            packet.routing_header.hop_index += 1;

            if let Err(e) = forward_channel.send(packet.clone()) {
                error!("Error sending packet: {}", e);
                if let Err(e) = self.controller_send.send(DroneEvent::PacketDropped(packet)) {
                    error!("Error sending PacketDropped event: {}", e);
                }
            } else if let Err(e) = self.controller_send.send(DroneEvent::PacketSent(packet)) {
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
    }

    fn return_nack(&self, packet: &Packet, nack_type: NackType) {
        info!(
            "Returning NACK to sender '{:?}' from '{}' with reason '{:?}'",
            Self::get_source(packet),
            self.id,
            nack_type
        );

        // reverse the hops list to get new path
        let hops = packet
            .routing_header
            .hops
            .split_at(packet.routing_header.hop_index + 1)
            .0
            .iter()
            .rev()
            .cloned()
            .collect();

        // build the NACK packet
        let nack = Packet {
            pack_type: PacketType::Nack(Nack {
                fragment_index: if let PacketType::MsgFragment(fragment) = &packet.pack_type {
                    fragment.fragment_index
                } else {
                    0
                },
                nack_type,
            }),
            routing_header: SourceRoutingHeader { hops, hop_index: 0 },
            session_id: packet.session_id,
        };

        // now route the NACK packet
        self.route_packet(nack);
    }

    fn return_flood_response(
        &mut self,
        flood_request: FloodRequest,
        neighbour: NodeId,
        session_id: u64,
    ) {
        let hops = flood_request
            .path_trace
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

    fn handle_flood_request(&mut self, packet: Packet) {
        let mut flood_request = match packet.pack_type {
            PacketType::FloodRequest(flood_request) => flood_request,
            _ => unreachable!(),
        };

        debug!(
            "Drone '{}' handling flood request with id '{}'",
            self.id, flood_request.flood_id
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
            self.return_flood_response(flood_request, sender_id, packet.session_id);
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
                            session_id: packet.session_id,
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
                self.return_flood_response(flood_request, sender_id, packet.session_id);
            }
        }
    }
}
