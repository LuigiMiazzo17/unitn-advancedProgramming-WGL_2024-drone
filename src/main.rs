use crossbeam::channel::unbounded;
use drone::RustDrone;
use log::debug;
use simulation_controller::{parse_config, SimulationController};
use std::collections::HashMap;
use std::thread;
use wg_2024::drone::Drone;
use wg_2024::drone::DroneOptions;

fn main() {
    env_logger::init();

    let config = parse_config("examples/config/base.toml");
    debug!("Loaded config: {:?}", config);

    let mut controller_drones = HashMap::new();
    let (node_event_send, node_event_recv) = unbounded();

    let mut packet_channels = HashMap::new();
    for drone in config.drone.iter() {
        packet_channels.insert(drone.id, unbounded());
    }
    for client in config.client.iter() {
        packet_channels.insert(client.id, unbounded());
    }
    for server in config.server.iter() {
        packet_channels.insert(server.id, unbounded());
    }

    let mut handles = Vec::new();
    for drone in config.drone.into_iter() {
        // controller
        let (controller_drone_send, controller_drone_recv) = unbounded();
        controller_drones.insert(drone.id, controller_drone_send);
        let node_event_send = node_event_send.clone();
        // packet
        let packet_recv = packet_channels[&drone.id].1.clone();
        let packet_send = drone
            .connected_node_ids
            .into_iter()
            .map(|id| (id, packet_channels[&id].0.clone()))
            .collect();

        handles.push(thread::spawn(move || {
            let mut drone = RustDrone::new(DroneOptions {
                id: drone.id,
                controller_recv: controller_drone_recv,
                controller_send: node_event_send,
                packet_recv,
                packet_send,
                pdr: drone.pdr,
            });

            drone.run();
        }));
    }
    let mut controller = SimulationController {
        drones: controller_drones,
        node_event_recv,
    };
    controller.crash_all();

    while let Some(handle) = handles.pop() {
        handle.join().unwrap();
    }
}
