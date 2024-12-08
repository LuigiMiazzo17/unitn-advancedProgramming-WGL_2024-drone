use wg_2024_rust::network_initializer::parse_config;
use wg_2024_rust::network_initializer::spawn_network;
use wg_2024_rust::simulation_controller::SimulationController;

fn main() {
    env_logger::init();

    let config = parse_config("examples/config/base.toml");

    let (controller_drones, node_event_recv, mut handles) = spawn_network(config);

    let mut controller = SimulationController {
        drones: controller_drones,
        node_event_recv,
    };
    controller.crash_all();

    while let Some(handle) = handles.pop() {
        handle.join().unwrap();
    }
}
