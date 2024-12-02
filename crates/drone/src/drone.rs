use crossbeam::channel::{select_biased, Receiver, Sender};
use rand::Rng;
use std::collections::HashMap;

use wg_2024::controller::{DroneCommand, NodeEvent};
use wg_2024::drone::{Drone, DroneOptions};
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{Nack, NackType, Packet, PacketType};

/// Example of drone implementation
pub struct RustDrone {
    id: NodeId,
    controller_send: Sender<NodeEvent>,
    controller_recv: Receiver<DroneCommand>,
    packet_recv: Receiver<Packet>,
    pdr: f32,
    packet_send: HashMap<NodeId, Sender<Packet>>,
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
        }
    }

    fn run(&mut self) {
        loop {
            select_biased! {
                recv(self.controller_recv) -> command => {
                    if let Ok(command) = command {
                        if let DroneCommand::Crash = command {
                            println!("drone {} crashed", self.id);
                            break;
                        }
                        self.handle_command(command);
                    }
                }
                recv(self.packet_recv) -> packet => {
                    if let Ok(packet) = packet {
                        self.handle_packet(packet);
                    }
                },
            }
        }
    }
}

impl RustDrone {
    fn handle_packet(&mut self, packet: Packet) {
        match packet.pack_type {
            PacketType::Nack(_) => self.forward_packet(packet),
            PacketType::Ack(_) => self.forward_packet(packet),
            PacketType::MsgFragment(_) => self.forward_packet(packet),
            PacketType::FloodRequest(_flood_request) => todo!(),
            PacketType::FloodResponse(_flood_response) => todo!(),
        }
    }

    fn handle_command(&mut self, command: DroneCommand) {
        match command {
            DroneCommand::AddSender(_node_id, _sender) => todo!(),
            DroneCommand::SetPacketDropRate(_pdr) => todo!(),
            DroneCommand::Crash => unreachable!(),
        }
    }

    fn forward_packet(&self, mut packet: Packet) {
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
                    packet.routing_header.hop_index += 1;
                    sender.send(packet).unwrap();
                } else {
                    // drop the packet
                    self.return_nack(&packet, NackType::Dropped);
                }
            } else {
                // next hop is not in the list of connected nodes
                self.return_nack(&packet, NackType::ErrorInRouting(*next_hop));
            }
        } else {
            // malformed packet, the destination is the drone itself
            self.return_nack(&packet, NackType::DestinationIsDrone);
        }
    }

    fn return_nack(&self, packet: &Packet, nack_type: NackType) {
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
}
