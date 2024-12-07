#[allow(clippy::module_inception)]
mod drone;

pub use drone::*;

#[cfg(test)]
mod tests;
