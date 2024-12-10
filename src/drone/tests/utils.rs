use super::super::drone::*;

use crossbeam::channel::{unbounded, Receiver, Sender};
use rand::Rng;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};
use wg_2024::packet::Packet;

use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::drone::Drone;
use wg_2024::network::NodeId;

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

    let (controller_send, controller_recv) = unbounded();

    // provision drones
    for (drone_id, (pdr, _)) in config.iter() {
        let pdr = *pdr;
        let drone_id = *drone_id;
        let (d_send, d_recv) = unbounded();
        let (d_command_send, d_command_recv) = unbounded();
        let clone_send = controller_send.clone();

        let d_t = thread::Builder::new()
            .name(format!("drone{}", drone_id))
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

        hm.insert(drone_id, (d_t, d_send, d_command_send));
    }

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

    let timeout = Duration::from_secs(1);
    let start_time = Instant::now();

    // check if all drones have finished, panic if not
    while start_time.elapsed() < timeout {
        if hm.iter().all(|(_, (drone_t, _, _))| drone_t.is_finished()) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("Not all drones have finished in time");
}
