mod units;
mod utils;

use std::time::Duration;

const DRONE_CRASH_TIMEOUT: Duration = Duration::from_millis(150);
const DRONE_CRASH_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_PACKET_WAIT: Duration = Duration::from_millis(150);
