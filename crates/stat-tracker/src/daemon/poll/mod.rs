//! The poll tick.

mod cadence;
mod outcome;
#[cfg(test)]
mod tests_cadence;
#[cfg(test)]
mod tests_reader_switch;

pub(crate) use cadence::*;
pub(crate) use outcome::*;
