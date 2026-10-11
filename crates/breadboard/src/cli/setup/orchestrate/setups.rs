//! An issue's start while it runs (#272, setups.mjs): core's claims
//! (`breadboard_core::setups`, one implementation with the handover's), in
//! this repo, the claim's file also held for `proc`'s failure path: a
//! process that ends through `fail` runs no drops.

use crate::cli::setup::proc;
use crate::cli::setup::repo;
use breadboard_core::setups::{self, Setup};
use std::path::PathBuf;

pub use breadboard_core::setups::setup_text;

pub fn setups_dir(name: &str) -> PathBuf {
    setups::setups_dir(&repo::main_dir(), name)
}

/// A claim on the start of an issue, released by `release`, when dropped or when this process fails.
pub struct Claim(setups::Claim);

impl Claim {
    pub fn release(&self) {
        if let Some(file) = self.0.file() {
            proc::unhold(file);
        }
        self.0.release();
    }
}

/// Claims the start of `number` for this process: Ok, or Err(the setup holding it, when it can be read).
pub fn claim_setup(
    name: &str,
    number: i64,
    dir: &str,
    to: Option<&str>,
    from: Option<&str>,
) -> Result<Claim, Option<Setup>> {
    match setups::claim_setup(&repo::main_dir(), name, number, dir, to, from) {
        Ok(Ok(claim)) => {
            if let Some(file) = claim.file() {
                proc::hold(file);
            }
            Ok(Claim(claim))
        }
        Ok(Err(holder)) => Err(holder),
        Err(_) => Err(None),
    }
}

/// What holds a start: its setup in words, or that one does.
pub fn holder_text(holder: Option<&Setup>) -> String {
    holder.map_or_else(|| "by another start".to_string(), setup_text)
}
