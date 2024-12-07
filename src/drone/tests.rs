use super::*;
use crossbeam::channel::unbounded;
use crossbeam::channel::Sender;
use rand::Rng;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};
use wg_2024::controller::DroneCommand;
use wg_2024::drone::Drone;
use wg_2024::network::SourceRoutingHeader;
use wg_2024::packet::Ack;
use wg_2024::packet::Fragment;
use wg_2024::packet::{Nack, NackType, Packet, PacketType};
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

    terminate_and_assert_drone_quit(drone_t, m_controller_send);
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

    terminate_and_assert_drone_quit(drone_t, update_send);
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

    terminate_and_assert_drone_quit(d_t, d_command_send);
    terminate_and_assert_drone_quit(d2_t, d2_command_send);
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

    terminate_and_assert_drone_quit(d_t, d_command_send);
    terminate_and_assert_drone_quit(d2_t, d2_command_send);
}
