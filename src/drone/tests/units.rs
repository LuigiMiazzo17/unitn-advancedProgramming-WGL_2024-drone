use super::super::drone::*;
use super::utils::{
    generate_random_payload, provision_drones_from_config, send_command_to_drone,
    send_packet_to_drone, terminate_env,
};

use crossbeam::channel::unbounded;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use wg_2024::controller::DroneCommand;
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{
    Ack, FloodRequest, FloodResponse, Fragment, Nack, NackType, NodeType, Packet, PacketType,
};
use wg_2024::tests::*;

#[test]
fn drone_crashes_upon_cmd() {
    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![]));

    let (_, env) = provision_drones_from_config(config);

    terminate_env(env);
}

#[test]
fn drone_forwards_fragment() {
    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![]));
    let (d2_send, d2_recv) = unbounded();

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 11, DroneCommand::AddSender(12, d2_send.clone()));

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
            hops: vec![11, 12],
            hop_index: 0,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, 11, sending_packet.clone());

    let mut expected_packet = sending_packet;
    // The packet should have been forwarded to the neighbour
    expected_packet.routing_header.hop_index = 1;

    assert_eq!(
        d2_recv.recv_timeout(Duration::from_secs(1)).unwrap(),
        expected_packet
    );

    terminate_env(env);
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
fn wrap_generic_chain_fragment_drop() {
    generic_chain_fragment_drop::<RustDrone>();
}

#[test]
fn wrap_generic_chain_fragment_ack() {
    generic_chain_fragment_ack::<RustDrone>();
}

#[test]
fn generic_chain_fragment_drop_2() {
    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![12]));
    config.insert(12, (1.0, vec![11]));
    let (c_send, c_recv) = unbounded();
    let (s_send, _s_recv) = unbounded();

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 11, DroneCommand::AddSender(1, c_send.clone()));
    send_command_to_drone(&env, 12, DroneCommand::AddSender(21, s_send.clone()));

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

    send_packet_to_drone(&env, 11, msg.clone());

    assert_eq!(
        c_recv.recv_timeout(Duration::from_secs(1)).unwrap(),
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

    terminate_env(env);
}

#[test]
fn round_trip_message() {
    let (c_send, c_recv) = unbounded();
    let (s_send, s_recv) = unbounded();

    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![12]));
    config.insert(12, (0.0, vec![11]));

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 11, DroneCommand::AddSender(1, c_send.clone()));
    send_command_to_drone(&env, 12, DroneCommand::AddSender(21, s_send.clone()));

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
    send_packet_to_drone(&env, 11, msg.clone());

    msg.routing_header.hop_index = 3;
    // Server receives the fragment
    assert_eq!(s_recv.recv_timeout(Duration::from_secs(1)).unwrap(), msg);

    let mut ack = Packet {
        pack_type: PacketType::Ack(Ack { fragment_index: 0 }),
        routing_header: SourceRoutingHeader {
            hops: vec![21, 12, 11, 1],
            hop_index: 1,
        },
        session_id: msg.session_id,
    };

    // Server sends ack to the drone
    send_packet_to_drone(&env, 12, ack.clone());

    ack.routing_header.hop_index = 3;
    // Client receives the ack
    assert_eq!(c_recv.recv_timeout(Duration::from_secs(1)).unwrap(), ack);

    send_packet_to_drone(&env, 11, ack.clone());
}

#[test]
fn return_flood_response_with_one_neighbour() {
    let (c_send, c_recv) = unbounded();

    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![12]));
    config.insert(12, (0.0, vec![11]));

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 11, DroneCommand::AddSender(1, c_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();
    let flood_id = rand::thread_rng().gen::<u64>();

    let sending_flood_request = Packet {
        pack_type: PacketType::FloodRequest(FloodRequest {
            flood_id,
            initiator_id: 1,
            path_trace: vec![(1, NodeType::Client)],
        }),
        routing_header: SourceRoutingHeader {
            hops: Vec::new(),
            hop_index: 0,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, 11, sending_flood_request.clone());

    let expected_packet = Packet {
        pack_type: PacketType::FloodResponse(FloodResponse {
            flood_id,
            path_trace: vec![
                (1, NodeType::Client),
                (11, NodeType::Drone),
                (12, NodeType::Drone),
            ],
        }),
        routing_header: SourceRoutingHeader {
            hops: vec![12, 11, 1],
            hop_index: 2,
        },
        session_id,
    };

    assert_eq!(
        c_recv.recv_timeout(Duration::from_secs(1)).unwrap(),
        expected_packet
    );

    terminate_env(env);
}

#[test]
fn multiple_flood_requests_encounter() {
    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![12, 13]));
    config.insert(12, (0.0, vec![11, 14]));
    config.insert(13, (0.0, vec![11, 14]));
    config.insert(14, (0.0, vec![12, 13]));
    let (c_send, c_recv) = unbounded();

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 11, DroneCommand::AddSender(1, c_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();
    let flood_id = rand::thread_rng().gen::<u64>();

    let sending_flood_request = Packet {
        pack_type: PacketType::FloodRequest(FloodRequest {
            flood_id,
            initiator_id: 1,
            path_trace: vec![(1, NodeType::Client)],
        }),
        routing_header: SourceRoutingHeader {
            hops: Vec::new(),
            hop_index: 0,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, 11, sending_flood_request.clone());

    let mut expected_network_config = HashMap::new();

    expected_network_config.insert(1, vec![(11, NodeType::Drone)]);
    expected_network_config.insert(
        11,
        vec![
            (12, NodeType::Drone),
            (13, NodeType::Drone),
            (1, NodeType::Client),
        ],
    );
    expected_network_config.insert(12, vec![(11, NodeType::Drone), (14, NodeType::Drone)]);
    expected_network_config.insert(13, vec![(11, NodeType::Drone), (14, NodeType::Drone)]);
    expected_network_config.insert(14, vec![(12, NodeType::Drone), (13, NodeType::Drone)]);

    fn insert_hop(
        network_config: &mut HashMap<NodeId, Vec<(NodeId, NodeType)>>,
        node: NodeId,
        hop: (NodeId, NodeType),
    ) {
        if let Some(hops) = network_config.get_mut(&node) {
            if !hops.contains(&hop) {
                hops.push(hop);
            }
        } else {
            network_config.insert(node, vec![hop]);
        }
    }

    let mut flood_responses = Vec::new();

    while let Ok(packet) = c_recv.recv_timeout(Duration::from_secs(1)) {
        flood_responses.push(packet);
    }

    let mut received_network_config = HashMap::new();

    assert_eq!(flood_responses.len(), 2);

    for packet in flood_responses {
        if let PacketType::FloodResponse(flood_response) = packet.pack_type {
            assert_eq!(flood_response.flood_id, flood_id);

            for (i, (hop, _)) in flood_response.path_trace.clone().into_iter().enumerate() {
                if i != flood_response.path_trace.len() - 1 {
                    if let Some(next_hop) = flood_response.path_trace.get(i + 1) {
                        insert_hop(&mut received_network_config, hop, *next_hop);
                    }
                }

                if i != 0 {
                    if let Some(prev_hop) = flood_response.path_trace.get(i - 1) {
                        insert_hop(&mut received_network_config, hop, *prev_hop);
                    }
                }
            }
        } else {
            panic!("Received packet was not a FloodResponse");
        }
    }

    for (node, hops) in expected_network_config {
        let expected_hs = HashSet::<NodeId>::from_iter(hops.iter().map(|(id, _)| *id));
        let received_hs = HashSet::<NodeId>::from_iter(
            received_network_config
                .get(&node)
                .unwrap()
                .iter()
                .map(|(id, _)| *id),
        );

        assert_eq!(expected_hs, received_hs);
    }

    terminate_env(env);
}
