use crossbeam::channel::{select, select_biased, Receiver, Sender};
use log::{debug, error, info, trace, warn};
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::thread;

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
    seen_flood_requests: HashSet<u64>,
    log_target: String,
    state: DroneState,
}

enum CommandResult {
    Ok,
    Quit,
}

#[derive(Debug)]
enum DroneState {
    Created,
    Running,
    Crashing,
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
            seen_flood_requests: HashSet::new(),
            log_target: format!("drone-{}", id),
            state: DroneState::Created,
        }
    }

    fn run(&mut self) {
        trace!(target: &self.log_target, "Drone '{}' has started", self.id);
        self.state = DroneState::Running;

        loop {
            select_biased! {
                recv(self.controller_recv) -> command => {
                    if let Ok(command) = command {
                        match self.handle_command(command) {
                            CommandResult::Quit => break,
                            CommandResult::Ok => {}
                        }
                    }
                },
                recv(self.packet_recv) -> packet => {
                    if let Ok(packet) = packet {
                        self.handle_packet(packet);
                    }
                    else {
                        error!(target: &self.log_target, "Drone '{}' failed to receive packet, crashing", self.id);
                        break; // channel closed, exit the loop
                    }
                },
            }
        }

        if matches!(self.state, DroneState::Crashing) {
            trace!(target: &self.log_target, "Drone '{}' is crashing state, waiting for Reciver to be closed", self.id);
            loop {
                select! {
                    recv(self.packet_recv) -> packet => {
                        if let Ok(packet) = packet {
                            self.handle_packet(packet);
                        }
                        else {
                            debug!(target: &self.log_target, "Drone '{}' Reciver closed, stopping", self.id);
                            break;
                        }
                    }
                }
            }
        }
        trace!(target: &self.log_target, "Drone '{}' has succesfully stopped", self.id);
    }
}

impl RustDrone {
    fn handle_packet(&mut self, packet: Packet) {
        trace!(target: &self.log_target,
            "Drone '{}' on thread '{}' with state '{:?}' recived packet: {:?}",
            self.id,
            thread::current().name().unwrap_or("unnamed"),
            self.state,
            packet
        );

        // drone is crashing, ignore all packets
        if matches!(self.state, DroneState::Crashing) {
            match packet.pack_type {
                PacketType::FloodResponse(_) => {}
                PacketType::Nack(_) => {}
                PacketType::Ack(_) => {}
                PacketType::FloodRequest(_) => return,
                _ => self.return_nack(&packet, NackType::ErrorInRouting(self.id)),
            };
        };

        match packet.pack_type {
            PacketType::FloodRequest(_) => self.handle_flood_request(packet),
            _ => {
                let current_hop = match Self::get_current_hop(&packet) {
                    Some(current_hop) => current_hop,
                    None => {
                        // we received a packet with no current hop
                        error!(target: &self.log_target, "Recived packet with no current hop");
                        return;
                    }
                };

                if current_hop == self.id {
                    // handle correctly the packet
                    debug!(target: &self.log_target, "Drone '{}' processing packet", self.id);
                    self.route_packet(packet)
                } else {
                    // we received a packet with wrong current hop
                    warn!(target: &self.log_target,
                        "Drone '{}' received packet with wrong current hop '{}'",
                        self.id, current_hop
                    );

                    let mut packet = packet;
                    packet.routing_header.hops[packet.routing_header.hop_index] = self.id;

                    self.return_nack(&packet, NackType::UnexpectedRecipient(self.id))
                }
            }
        }
    }

    fn handle_command(&mut self, command: DroneCommand) -> CommandResult {
        match command {
            DroneCommand::AddSender(node_id, sender) => {
                info!(target: &self.log_target, "Drone '{}' connected to '{}'", self.id, node_id);
                self.packet_send.insert(node_id, sender);
                CommandResult::Ok
            }
            DroneCommand::RemoveSender(node_id) => {
                info!(target: &self.log_target, "Drone '{}' disconnected from '{}'", self.id, node_id);
                if self.packet_send.remove(&node_id).is_none() {
                    warn!(target: &self.log_target,
                        "Drone '{}' tried to disconnect from '{}', but it was not connected",
                        self.id, node_id
                    );
                }
                CommandResult::Ok
            }
            DroneCommand::SetPacketDropRate(pdr) => {
                info!(target: &self.log_target, "Drone '{}' set PDR to {}", self.id, pdr);
                self.pdr = pdr;
                CommandResult::Ok
            }
            DroneCommand::Crash => {
                info!(target: &self.log_target, "Drone '{}' recived crash", self.id);
                self.state = DroneState::Crashing;
                CommandResult::Quit
            }
        }
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

    fn deliver_packet(&mut self, channel: &Sender<Packet>, sender_id: NodeId, packet: Packet) {
        if let Err(e) = channel.try_send(packet.clone()) {
            // if error indicates that the receiver has been dropped, we should remove the sender
            if matches!(e, crossbeam::channel::TrySendError::Disconnected(_)) {
                if self.packet_send.remove(&sender_id).is_none() {
                    error!(target: &self.log_target,
                        "Drone '{}' tried to disconnect from '{}', but it was not connected",
                        self.id, sender_id
                    );
                }
                warn!(target: &self.log_target,
                    "Drone '{}' disconnected from '{}' due to channel disconnected",
                    self.id, sender_id
                );
                self.return_nack(&packet, NackType::ErrorInRouting(sender_id));
            } else {
                error!(target: &self.log_target,
                    "Drone '{}' failed to send packet to channel: {}",
                    self.id, e
                );
            }

            if let Err(e) = self.controller_send.send(DroneEvent::PacketDropped(packet)) {
                error!(target: &self.log_target,
                    "Drone '{}' failed to send PacketDropped event to controller: {}",
                    self.id, e
                );
            }
        } else if let Err(e) = self.controller_send.send(DroneEvent::PacketSent(packet)) {
            error!(target: &self.log_target,
                "Drone '{}' failed to send PacketSent event to controller: {}",
                self.id, e
            );
        }
    }

    fn route_packet(&mut self, mut packet: Packet) {
        // check if the packet has another hop
        let next_hop = match Self::get_next_hop(&packet) {
            Some(next_hop) => next_hop,
            None => {
                // the destination is the drone itself
                if !matches!(&packet.pack_type, PacketType::Nack(_)) {
                    warn!(target: &self.log_target, "Destination is drone '{}' itself", self.id);
                    self.return_nack(&packet, NackType::DestinationIsDrone);
                } else {
                    debug!(target: &self.log_target,
                        "Packet is a Nack, destination is drone '{}' itself",
                        self.id
                    );
                };
                return;
            }
        };

        // check if the next hop is in the list of connected nodes
        let forward_channel = match self.packet_send.get(&next_hop) {
            Some(sender) => sender.clone(),
            None => {
                // next hop is not in the list of connected nodes
                warn!(target: &self.log_target,
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
            debug!(target: &self.log_target, "Drone '{}' forwarding packet to '{}'", self.id, next_hop);
            packet.routing_header.hop_index += 1;

            self.deliver_packet(&forward_channel, next_hop, packet)
        } else {
            // drop the packet
            info!(target: &self.log_target, "Packet has been dropped from node '{}'", self.id);
            if let Err(e) = self
                .controller_send
                .send(DroneEvent::PacketDropped(packet.clone()))
            {
                error!(target: &self.log_target,
                    "Drone '{}' failed to send PacketDropped event: {}",
                    self.id, e
                );
            }
            self.return_nack(&packet, NackType::Dropped);
        }
    }

    fn return_nack(&mut self, packet: &Packet, nack_type: NackType) {
        info!(target: &self.log_target,
            "Returning NACK to sender '{:?}' from '{}' with reason '{:?}'",
            packet.routing_header.hops.first(),
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

        let sender = match self.packet_send.get(&neighbour) {
            Some(sender) => sender.clone(),
            None => {
                error!(target: &self.log_target,
                    "Drone '{}' tried to return flood response to '{}', but it was not connected to it",
                    self.id, neighbour
                );
                return;
            }
        };

        let flood_response = Packet {
            pack_type: PacketType::FloodResponse(FloodResponse {
                flood_id: flood_request.flood_id,
                path_trace: flood_request.path_trace,
            }),
            routing_header: SourceRoutingHeader { hops, hop_index: 1 },
            session_id,
        };

        trace!(target: &self.log_target,
            "Drone '{}' returning flood response to '{}'",
            self.id,
            neighbour
        );
        self.deliver_packet(&sender, neighbour, flood_response);
    }

    fn handle_flood_request(&mut self, packet: Packet) {
        let mut flood_request = match packet.pack_type {
            PacketType::FloodRequest(flood_request) => flood_request,
            _ => unreachable!(),
        };

        trace!(target: &self.log_target,
            "Drone '{}' handling flood request with id '{}'",
            self.id,
            flood_request.flood_id
        );

        let sender_id = match flood_request.path_trace.last() {
            Some(a) => a.0,
            None => {
                error!(target: &self.log_target,
                    "Path trace in flood request {} is empty",
                    flood_request.flood_id
                );
                return;
            }
        };

        flood_request.path_trace.push((self.id, NodeType::Drone));

        if self.seen_flood_requests.contains(&flood_request.flood_id) {
            // we have already seen this flood request
            debug!(target: &self.log_target,
                "Drone '{}' has already seen flood request with id '{}'",
                self.id, flood_request.flood_id
            );
            self.return_flood_response(flood_request, sender_id, packet.session_id);
        } else {
            // never seen this flood request
            debug!(target: &self.log_target,
                "Drone '{}' handling flood request with id '{}' for the first time",
                self.id, flood_request.flood_id
            );
            self.seen_flood_requests.insert(flood_request.flood_id);

            if self.packet_send.len() > 1 {
                // we have more than one neighbour, we need to forward the flood request to all but one
                debug!(target: &self.log_target,
                    "Drone '{}' has more than one neighbour, forwarding flood request to all but '{}'",
                    self.id, sender_id
                );

                for (neighbour, sender) in self.packet_send.clone().iter() {
                    if *neighbour == sender_id {
                        continue;
                    }

                    trace!(target: &self.log_target,
                        "Drone '{}' forwarding flood request to '{}'",
                        self.id,
                        neighbour
                    );

                    self.deliver_packet(
                        sender,
                        *neighbour,
                        Packet {
                            pack_type: PacketType::FloodRequest(flood_request.clone()),
                            routing_header: SourceRoutingHeader {
                                hops: Vec::new(),
                                hop_index: 0,
                            },
                            session_id: packet.session_id,
                        },
                    );
                }
            } else {
                // we have only one neighbour, we can return the flood response
                debug!(target: &self.log_target,
                    "Drone '{}' has no other neighbour, returning a flood response to '{}'",
                    self.id, sender_id
                );
                self.return_flood_response(flood_request, sender_id, packet.session_id);
            }
        }
    }
}
