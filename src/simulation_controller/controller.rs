use crossbeam::channel::{Receiver, Sender};
use std::collections::HashMap;
use std::fs;

use wg_2024::config::Config;
use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::network::NodeId;

pub struct SimulationController {
    pub drones: HashMap<NodeId, Sender<DroneCommand>>,
    pub node_event_recv: Receiver<DroneEvent>,
}

impl SimulationController {
    pub fn crash_all(&mut self) {
        for (_, sender) in self.drones.iter() {
            sender.send(DroneCommand::Crash).unwrap();
        }
    }
}

pub fn parse_config(file: &str) -> Config {
    let file_str = fs::read_to_string(file).unwrap();
    toml::from_str(&file_str).unwrap()
}
