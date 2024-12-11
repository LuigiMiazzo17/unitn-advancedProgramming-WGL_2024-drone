mod units;
mod utils;

use std::time::Duration;

const DRONE_CRASH_TIMEOUT: Duration = Duration::from_millis(150);
const DRONE_CRASH_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_PACKET_WAIT_TIMEOUT: Duration = Duration::from_millis(150);
const MAX_RANDOM_DRONES: u8 = 50;
const AVG_RANDOM_NEIGHBOUR_FOR_DRONE: u8 = 15;
