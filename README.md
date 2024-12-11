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

# Customer Support

For any question, issues or feedback, please contact us at this [Service desk](https://sbling.atlassian.net/servicedesk/customer/portal/2) or contact us on Telegram.
