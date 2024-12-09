use super::*;
use crossbeam::channel::{unbounded, Sender};
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use wg_2024::controller::DroneCommand;
use wg_2024::drone::Drone;
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{
    Ack, FloodRequest, FloodResponse, Fragment, Nack, NackType, NodeType, Packet, PacketType,
};
use wg_2024::tests::*;

fn terminate_and_assert_drone_quit(
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
    let (d_command_send, d_command_recv) = unbounded();
    let (_, d_recv) = unbounded();

    let d_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                11,
                unbounded().0,
                d_command_recv,
                d_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        })
        .unwrap();

    terminate_and_assert_drone_quit(d_t, d_command_send);
}

#[test]
fn drone_forwards_fragment() {
    let (d_send, d_recv) = unbounded();
    let (d2_send, d2_recv) = unbounded();

    let (d_command_send, d_command_recv) = unbounded();

    let d_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                11,
                unbounded().0,
                d_command_recv,
                d_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        })
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(12, d2_send))
        .unwrap();

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
    d_send.send(sending_packet.clone()).unwrap();

    let mut expected_packet = sending_packet;
    // The packet should have been forwarded to the neighbour
    expected_packet.routing_header.hop_index = 1;

    assert_eq!(
        d2_recv.recv_timeout(Duration::from_secs(1)).unwrap(),
        expected_packet
    );

    terminate_and_assert_drone_quit(d_t, d_command_send);
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
    let (c_send, c_recv) = unbounded();
    let (s_send, _s_recv) = unbounded();

    let (d_send, d_recv) = unbounded();
    let (d2_send, d2_recv) = unbounded();

    let (d_command_send, d_command_recv) = unbounded();
    let (d2_command_send, d2_command_recv) = unbounded();

    let d_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                11,
                unbounded().0,
                d_command_recv,
                d_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        })
        .unwrap();

    let d2_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                12,
                unbounded().0,
                d2_command_recv,
                d2_recv,
                HashMap::new(),
                1.0,
            );
            drone.run();
        })
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(1, c_send.clone()))
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(12, d2_send.clone()))
        .unwrap();

    d2_command_send
        .send(DroneCommand::AddSender(11, d_send.clone()))
        .unwrap();

    d2_command_send
        .send(DroneCommand::AddSender(21, s_send.clone()))
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

    d_send.send(msg.clone()).unwrap();

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

    terminate_and_assert_drone_quit(d_t, d_command_send);
    terminate_and_assert_drone_quit(d2_t, d2_command_send);
}

#[test]
fn round_trip_message() {
    let (c_send, c_recv) = unbounded();
    let (s_send, s_recv) = unbounded();

    let (d_send, d_recv) = unbounded();
    let (d12_send, d12_recv) = unbounded();

    let (d_command_send, d_command_recv) = unbounded();
    let (d2_command_send, d2_command_recv) = unbounded();

    let d_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                11,
                unbounded().0,
                d_command_recv,
                d_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        })
        .unwrap();

    let d2_t = thread::Builder::new()
        .name("drone2".to_string())
        .spawn(move || {
            let mut drone2 = RustDrone::new(
                12,
                unbounded().0,
                d2_command_recv,
                d12_recv,
                HashMap::new(),
                0.0,
            );
            drone2.run();
        })
        .unwrap();

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
    d12_send.send(ack.clone()).unwrap();

    ack.routing_header.hop_index = 3;
    // Client receives the ack
    assert_eq!(c_recv.recv_timeout(Duration::from_secs(1)).unwrap(), ack);

    terminate_and_assert_drone_quit(d_t, d_command_send);
    terminate_and_assert_drone_quit(d2_t, d2_command_send);
}

#[test]
fn return_flood_response_with_one_neighbour() {
    let (c_send, c_recv) = unbounded();

    let (d_send, d_recv) = unbounded();
    let (d2_send, d2_recv) = unbounded();

    let (d_command_send, d_command_recv) = unbounded();
    let (d2_command_send, d2_command_recv) = unbounded();

    let d_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                11,
                unbounded().0,
                d_command_recv,
                d_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        })
        .unwrap();

    let d2_t = thread::Builder::new()
        .name("drone2".to_string())
        .spawn(move || {
            let mut drone2 = RustDrone::new(
                12,
                unbounded().0,
                d2_command_recv,
                d2_recv,
                HashMap::new(),
                0.0,
            );
            drone2.run();
        })
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(12, d2_send.clone()))
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(1, c_send.clone()))
        .unwrap();

    d2_command_send
        .send(DroneCommand::AddSender(11, d_send.clone()))
        .unwrap();

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
    d_send.send(sending_flood_request).unwrap();

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

    terminate_and_assert_drone_quit(d_t, d_command_send);
    terminate_and_assert_drone_quit(d2_t, d2_command_send);
}

#[test]
fn multiple_flood_requests_encounter() {
    let (c_send, c_recv) = unbounded();

    let (d_send, d_recv) = unbounded();
    let (d2_send, d2_recv) = unbounded();
    let (d3_send, d3_recv) = unbounded();
    let (d4_send, d4_recv) = unbounded();

    let (d_command_send, d_command_recv) = unbounded();
    let (d2_command_send, d2_command_recv) = unbounded();
    let (d3_command_send, d3_command_recv) = unbounded();
    let (d4_command_send, d4_command_recv) = unbounded();

    let d_t = thread::Builder::new()
        .name("drone1".to_string())
        .spawn(move || {
            let mut drone = RustDrone::new(
                11,
                unbounded().0,
                d_command_recv,
                d_recv,
                HashMap::new(),
                0.0,
            );
            drone.run();
        })
        .unwrap();

    let d2_t = thread::Builder::new()
        .name("drone2".to_string())
        .spawn(move || {
            let mut drone2 = RustDrone::new(
                12,
                unbounded().0,
                d2_command_recv,
                d2_recv,
                HashMap::new(),
                0.0,
            );
            drone2.run();
        })
        .unwrap();

    let d3_t = thread::Builder::new()
        .name("drone3".to_string())
        .spawn(move || {
            let mut drone3 = RustDrone::new(
                13,
                unbounded().0,
                d3_command_recv,
                d3_recv,
                HashMap::new(),
                0.0,
            );
            drone3.run();
        })
        .unwrap();

    let d4_t = thread::Builder::new()
        .name("drone4".to_string())
        .spawn(move || {
            let mut drone4 = RustDrone::new(
                14,
                unbounded().0,
                d4_command_recv,
                d4_recv,
                HashMap::new(),
                0.0,
            );
            drone4.run();
        })
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(1, c_send.clone()))
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(12, d2_send.clone()))
        .unwrap();

    d_command_send
        .send(DroneCommand::AddSender(13, d3_send.clone()))
        .unwrap();

    d2_command_send
        .send(DroneCommand::AddSender(11, d_send.clone()))
        .unwrap();

    d2_command_send
        .send(DroneCommand::AddSender(14, d4_send.clone()))
        .unwrap();

    d3_command_send
        .send(DroneCommand::AddSender(11, d_send.clone()))
        .unwrap();

    d3_command_send
        .send(DroneCommand::AddSender(14, d4_send.clone()))
        .unwrap();

    d4_command_send
        .send(DroneCommand::AddSender(12, d2_send.clone()))
        .unwrap();

    d4_command_send
        .send(DroneCommand::AddSender(13, d3_send.clone()))
        .unwrap();

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
    d_send.send(sending_flood_request).unwrap();

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

    terminate_and_assert_drone_quit(d_t, d_command_send);
    terminate_and_assert_drone_quit(d2_t, d2_command_send);
    terminate_and_assert_drone_quit(d3_t, d3_command_send);
    terminate_and_assert_drone_quit(d4_t, d4_command_send);
}
