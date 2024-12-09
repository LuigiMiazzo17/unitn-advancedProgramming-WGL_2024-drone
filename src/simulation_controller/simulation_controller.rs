use crossbeam::channel::{Receiver, Sender};
use std::collections::HashMap;
use wg_2024::controller::{DroneCommand, DroneEvent};
use wg_2024::network::NodeId;

pub struct SimulationController {
    pub drones: HashMap<NodeId, Sender<DroneCommand>>,
    pub node_event_recv: Receiver<DroneEvent>,
}

impl SimulationController {
    pub fn crash_all(&mut self) -> anyhow::Result<()> {
        for (_, sender) in self.drones.iter() {
            sender.send(DroneCommand::Crash)?;
        }
        Ok(())
    }
}
