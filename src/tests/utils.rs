use super::super::drone::*;
use super::*;

use crossbeam::channel::{unbounded, Receiver, Sender};
use log4rs_test_utils::test_logging::init_logging_once_for;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::thread;
use std::time::Instant;

use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::drone::Drone;
use wg_2024::network::NodeId;
use wg_2024::packet::{Packet, PacketType};

type Config = HashMap<NodeId, (f32, Vec<NodeId>)>;
type Environment = HashMap<NodeId, (thread::JoinHandle<()>, Sender<Packet>, Sender<DroneCommand>)>;

pub fn generate_random_payload() -> (u8, [u8; 128]) {
    let payload_len = rand::thread_rng().gen_range(1..128) as u8;
    let mut payload: [u8; 128] = [0; 128];
    let payload_vec = vec![rand::thread_rng().gen::<u8>(); payload_len as usize];

    for (i, byte) in payload_vec.iter().enumerate() {
        payload[i] = *byte;
    }

    (payload_len, payload)
}

pub fn send_command_to_drone(hm: &Environment, drone_id: NodeId, command: DroneCommand) {
    hm.get(&drone_id)
        .unwrap()
        .2
        .send(command)
        .expect("Failed to send command to drone");
}

pub fn send_packet_to_drone(hm: &Environment, drone_id: NodeId, packet: Packet) {
    hm.get(&drone_id)
        .unwrap()
        .1
        .send(packet)
        .expect("Failed to send packet to drone");
}

pub fn provision_drones_from_config(config: Config) -> (Receiver<DroneEvent>, Environment) {
    let mut hm = HashMap::new();
    let mut d_loggers_targets = Vec::new();

    let (controller_send, controller_recv) = unbounded();

    // provision drones
    for (drone_id, (pdr, _)) in config.iter() {
        let pdr = *pdr;
        let drone_id = *drone_id;
        let (d_send, d_recv) = unbounded();
        let (d_command_send, d_command_recv) = unbounded();
        let clone_send = controller_send.clone();

        let d_t = thread::Builder::new()
            .name(format!("drone-{}", drone_id))
            .spawn(move || {
                let mut drone = RustDrone::new(
                    drone_id,
                    clone_send,
                    d_command_recv,
                    d_recv,
                    HashMap::new(),
                    pdr,
                );
                drone.run();
            })
            .expect("Failed to spawn drone thread");

        d_loggers_targets.push(format!("drone-{}", drone_id));
        hm.insert(drone_id, (d_t, d_send, d_command_send));
    }
    let d_loggers_targets = d_loggers_targets
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<&str>>();

    init_logging_once_for(d_loggers_targets, log::LevelFilter::Trace, None);

    // join neighbours
    for (drone_id, (_, _, d_command_send)) in hm.iter() {
        let (_, neighbours) = &config[drone_id];

        for neighbour in neighbours {
            d_command_send
                .send(DroneCommand::AddSender(
                    *neighbour,
                    hm.get(neighbour).unwrap().1.clone(),
                ))
                .expect("Failed to send AddSender command to drone");
        }
    }

    (controller_recv, hm)
}

pub fn terminate_env(hm: Environment) {
    for (_, (drone_t, _, d_command_send)) in hm.iter() {
        assert!(!drone_t.is_finished());
        d_command_send
            .send(DroneCommand::Crash)
            .expect("Failed to send Crash command to drone");
    }

    let start_time = Instant::now();

    // check if all drones have finished, panic if not
    while start_time.elapsed() < DRONE_CRASH_TIMEOUT {
        if hm.iter().all(|(_, (drone_t, _, _))| drone_t.is_finished()) {
            return;
        }
        thread::sleep(DRONE_CRASH_POLL_INTERVAL);
    }

    panic!("Not all drones have finished in time");
}

pub fn generate_random_config() -> (u64, Config) {
    let seed: u64 = rand::thread_rng().gen();

    (seed, generate_random_config_from_seed(seed))
}

fn generate_random_config_from_seed(seed: u64) -> Config {
    let mut config = HashMap::new();

    let mut r = rand::rngs::StdRng::seed_from_u64(seed);

    let n_drones = r.gen_range(1..=MAX_RANDOM_DRONES);
    let additional_connections =
        r.gen_range(1..=AVG_RANDOM_NEIGHBOUR_FOR_DRONE) as u32 * n_drones as u32;

    for i in 0..n_drones {
        let mut neighbours = Vec::new();

        if i > 0 {
            neighbours.push(i - 1);
        }
        if i < n_drones - 1 {
            neighbours.push(i + 1);
        }
        config.insert(i as NodeId, (0.0, neighbours));
    }

    for _ in 0..additional_connections {
        let a = r.gen_range(0..n_drones);
        let b = r.gen_range(0..n_drones);

        if a != b && !config[&(a as NodeId)].1.contains(&(b as NodeId)) {
            config.get_mut(&(a as NodeId)).unwrap().1.push(b as NodeId);
            config.get_mut(&(b as NodeId)).unwrap().1.push(a as NodeId);
        }
    }

    config
}

pub fn parse_network_from_flood_responses(
    flood_responses: Vec<Packet>,
) -> HashMap<NodeId, Vec<NodeId>> {
    fn insert_hop(network_config: &mut HashMap<NodeId, Vec<NodeId>>, node: NodeId, hop: NodeId) {
        if let Some(hops) = network_config.get_mut(&node) {
            if !hops.contains(&hop) {
                hops.push(hop);
            }
        } else {
            network_config.insert(node, vec![hop]);
        }
    }

    let mut received_network_config = HashMap::new();

    for packet in flood_responses {
        if let PacketType::FloodResponse(flood_response) = packet.pack_type {
            for (i, (hop, _)) in flood_response.path_trace.clone().into_iter().enumerate() {
                if i != flood_response.path_trace.len() - 1 {
                    if let Some(next_hop) = flood_response.path_trace.get(i + 1) {
                        insert_hop(&mut received_network_config, hop, next_hop.0);
                    }
                }

                if i != 0 {
                    if let Some(prev_hop) = flood_response.path_trace.get(i - 1) {
                        insert_hop(&mut received_network_config, hop, prev_hop.0);
                    }
                }
            }
        } else {
            panic!("Received packet was not a FloodResponse");
        }
    }

    received_network_config
}
