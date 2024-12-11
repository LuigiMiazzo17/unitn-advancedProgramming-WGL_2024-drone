[![CI](https://github.com/LuigiMiazzo17/unitn-advancedProgramming-WGL_2024-drone/actions/workflows/ci.yaml/badge.svg)](https://github.com/LuigiMiazzo17/unitn-advancedProgramming-WGL_2024-drone/actions/workflows/ci.yaml) [![codecov](https://codecov.io/gh/LuigiMiazzo17/unitn-advancedProgramming-WGL_2024-drone/branch/master/graph/badge.svg?token=FLZ8SBSZ7U)](https://codecov.io/gh/LuigiMiazzo17/unitn-advancedProgramming-WGL_2024-drone)

# Advanced Programming Project - Group "Rust" - Drone

This project is part of the Advanced Programming course at the University of Trento.\
It involves creating a distributed simulation of a network with drones, clients, and servers communicating using a custom protocol.

# Usage

This repo only contains the code for the drone part of the project.\
To import this in your project, add the following under your dependencies in `Cargo.toml`:

```toml
[dependencies]
wg_2024-rust = { git = "https://github.com/LuigiMiazzo17/unitn-advancedProgramming-WGL_2024-drone.git" }
```

Then, just import the crate in your code:

```rust
use wg_2024_rust::drone::RustDrone;
```

Have fun!

# Loggers

Our project uses the `log` crate for logging.\
This means that, if you want to consume the logs, you need to set up a logger in your code.\
Here is an example of how to set up a simple logger that logs to stdout with the default log level (`INFO`):

```rust
fn main() {
    env_logger::init();
    // Your code here
}
```

You may change the default log level by updating the `RUST_LOG` environment variable to the desired level (e.g. `RUST_LOG=debug`).

Each drone will log its own messages to a specific log target, which follows the format `drone-{drone_id}`.\
This means that you can filter logs by this target to see only the logs of a specific drone.

You may also decide to completely ignore logs and in that case the performance impact, as stated in the `log` crate documentation, is negligible.

# Customer Support

For any question, issues or feedback, please contact us at this [Service desk](https://sbling.atlassian.net/servicedesk/customer/portal/2) or contact us on Telegram.
