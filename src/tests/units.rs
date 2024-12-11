use super::super::drone::*;
use super::utils::{
    generate_random_config, generate_random_payload, parse_network_from_flood_responses,
    provision_drones_from_config, send_command_to_drone, send_packet_to_drone, terminate_env,
};
use super::MAX_PACKET_WAIT_TIMEOUT;

use crossbeam::channel::unbounded;
use rand::Rng;
use std::collections::{HashMap, HashSet};

use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{
    Ack, FloodRequest, FloodResponse, Fragment, Nack, NackType, NodeType, Packet, PacketType,
};
use wg_2024::tests::*;

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
fn drone_crashes_upon_cmd() {
    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![]));

    let (_, env) = provision_drones_from_config(config);

    terminate_env(env);
}

#[test]
fn drone_adds_sender() {
    let mut config = HashMap::new();
    config.insert(11, (0.0, vec![]));

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 11, DroneCommand::AddSender(12, unbounded().0));

    terminate_env(env);
}

#[test]
fn drone_removes_sender() {
    let mut config = HashMap::new();
    config.insert(0, (0.0, vec![1]));
    config.insert(1, (0.0, vec![0]));

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 0, DroneCommand::RemoveSender(1));

    terminate_env(env);
}

#[test]
fn drone_updates_pdr() {
    let c_id = 100;
    let s_id = 200;
    let d_id = 0;
    let mut config = HashMap::new();
    config.insert(d_id, (0.0, vec![]));
    let (c_send, c_recv) = unbounded();
    let (s_send, _s_recv) = unbounded();

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, d_id, DroneCommand::AddSender(c_id, c_send.clone()));
    send_command_to_drone(&env, d_id, DroneCommand::AddSender(s_id, s_send.clone()));
    send_command_to_drone(&env, d_id, DroneCommand::SetPacketDropRate(1.0));

    let (payload_len, payload) = generate_random_payload();

    let msg = Packet {
        pack_type: PacketType::MsgFragment(Fragment {
            fragment_index: 0,
            total_n_fragments: 1,
            length: payload_len,
            data: payload,
        }),
        routing_header: SourceRoutingHeader {
            hops: vec![c_id, d_id, s_id],
            hop_index: 1,
        },
        session_id: 1,
    };

    send_packet_to_drone(&env, d_id, msg);

    let expected_packet = Packet {
        pack_type: PacketType::Nack(Nack {
            fragment_index: 0,
            nack_type: NackType::Dropped,
        }),
        routing_header: SourceRoutingHeader {
            hops: vec![d_id, c_id],
            hop_index: 1,
        },
        session_id: 1,
    };

    assert_eq!(
        c_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(),
        expected_packet
    );

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
        d2_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(),
        expected_packet
    );

    terminate_env(env);
}

#[test]
fn ack_messages_are_not_affected_by_pdr() {
    let d_id = 0;
    let c_id = 100;
    let s_id = 200;
    let mut config = HashMap::new();
    config.insert(d_id, (1.0, vec![]));
    let (c_send, _) = unbounded();
    let (s_send, s_recv) = unbounded();

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, d_id, DroneCommand::AddSender(c_id, c_send.clone()));
    send_command_to_drone(&env, d_id, DroneCommand::AddSender(s_id, s_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();

    let sending_packet = Packet {
        pack_type: PacketType::Ack(Ack { fragment_index: 0 }),
        routing_header: SourceRoutingHeader {
            hops: vec![c_id, d_id, s_id],
            hop_index: 1,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, d_id, sending_packet.clone());

    let mut expected_packet = sending_packet;
    // The packet should have been forwarded to the neighbour
    expected_packet.routing_header.hop_index = 2;

    assert_eq!(
        s_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(),
        expected_packet
    );

    terminate_env(env);
}

#[test]
fn nack_messages_are_not_affected_by_pdr() {
    let d_id = 0;
    let c_id = 100;
    let s_id = 200;
    let mut config = HashMap::new();
    config.insert(d_id, (1.0, vec![]));
    let (c_send, _) = unbounded();
    let (s_send, s_recv) = unbounded();

    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, d_id, DroneCommand::AddSender(c_id, c_send.clone()));
    send_command_to_drone(&env, d_id, DroneCommand::AddSender(s_id, s_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();

    let sending_packet = Packet {
        pack_type: PacketType::Nack(Nack {
            fragment_index: 0,
            nack_type: NackType::Dropped,
        }),
        routing_header: SourceRoutingHeader {
            hops: vec![c_id, d_id, s_id],
            hop_index: 1,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, d_id, sending_packet.clone());

    let mut expected_packet = sending_packet;
    // The packet should have been forwarded to the neighbour
    expected_packet.routing_header.hop_index = 2;

    assert_eq!(
        s_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(),
        expected_packet
    );

    terminate_env(env);
}

#[test]
fn controll_event_on_packet_sent() {
    let d_id = 0;
    let c_id = 100;
    let s_id = 200;
    let mut config = HashMap::new();
    config.insert(d_id, (0.0, vec![]));
    let (c_send, _) = unbounded();
    let (s_send, _s_send) = unbounded();

    let (controller_recv, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, d_id, DroneCommand::AddSender(c_id, c_send.clone()));
    send_command_to_drone(&env, d_id, DroneCommand::AddSender(s_id, s_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();

    let mut sending_packet = Packet {
        pack_type: PacketType::Nack(Nack {
            fragment_index: 0,
            nack_type: NackType::Dropped,
        }),
        routing_header: SourceRoutingHeader {
            hops: vec![c_id, d_id, s_id],
            hop_index: 1,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, d_id, sending_packet.clone());

    sending_packet.routing_header.hop_index = 2;
    let expected_packet = DroneEvent::PacketSent(sending_packet);

    assert_eq!(
        controller_recv
            .recv_timeout(MAX_PACKET_WAIT_TIMEOUT)
            .unwrap(),
        expected_packet
    );

    terminate_env(env);
}

#[test]
fn controll_event_on_packet_drop() {
    let d_id = 0;
    let c_id = 100;
    let s_id = 200;
    let mut config = HashMap::new();
    config.insert(d_id, (1.0, vec![]));
    let (c_send, _) = unbounded();
    let (s_send, _s_send) = unbounded();

    let (controller_recv, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, d_id, DroneCommand::AddSender(c_id, c_send.clone()));
    send_command_to_drone(&env, d_id, DroneCommand::AddSender(s_id, s_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();
    let (payload_size, payload) = generate_random_payload();

    let mut sending_packet = Packet {
        pack_type: PacketType::MsgFragment(Fragment {
            fragment_index: 0,
            total_n_fragments: 1,
            length: payload_size,
            data: payload,
        }),
        routing_header: SourceRoutingHeader {
            hops: vec![c_id, d_id, s_id],
            hop_index: 1,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, d_id, sending_packet.clone());

    sending_packet.routing_header.hop_index = 1;
    let expected_packet = DroneEvent::PacketDropped(sending_packet);

    assert_eq!(
        controller_recv
            .recv_timeout(MAX_PACKET_WAIT_TIMEOUT)
            .unwrap(),
        expected_packet
    );

    terminate_env(env);
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
        c_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(),
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
    assert_eq!(s_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(), msg);

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
    assert_eq!(c_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(), ack);

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
        c_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT).unwrap(),
        expected_packet
    );

    terminate_env(env);
}

#[test]
fn flood_request_on_big_network() {
    let (seed, config) = generate_random_config();
    println!("Seed: {}", seed);

    let c_id = 100;
    let (c_send, c_recv) = unbounded();

    let mut expected_config = config.clone();
    let (_, env) = provision_drones_from_config(config);

    send_command_to_drone(&env, 1, DroneCommand::AddSender(c_id, c_send.clone()));

    let session_id = rand::thread_rng().gen::<u64>();
    let flood_id = rand::thread_rng().gen::<u64>();

    let sending_flood_request = Packet {
        pack_type: PacketType::FloodRequest(FloodRequest {
            flood_id,
            initiator_id: c_id,
            path_trace: vec![(c_id, NodeType::Client)],
        }),
        routing_header: SourceRoutingHeader {
            hops: Vec::new(),
            hop_index: 0,
        },
        session_id,
    };

    // Send the packet to the drone
    send_packet_to_drone(&env, 1, sending_flood_request.clone());

    let mut flood_responses = Vec::new();

    while let Ok(packet) = c_recv.recv_timeout(MAX_PACKET_WAIT_TIMEOUT) {
        flood_responses.push(packet);
    }

    let received_network_config = parse_network_from_flood_responses(flood_responses);

    expected_config.get_mut(&1).unwrap().1.push(c_id);

    for (node_id, (_, hops)) in expected_config {
        let expected_hs = HashSet::<NodeId>::from_iter(hops.iter().copied());
        let received_hs = HashSet::<NodeId>::from_iter(
            received_network_config
                .get(&node_id)
                .unwrap()
                .iter()
                .copied(),
        );

        assert_eq!(expected_hs, received_hs);
    }

    terminate_env(env);
}
