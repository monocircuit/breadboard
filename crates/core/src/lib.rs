//! What every part of breadboard reads and writes. The data formats are
//! today's, read and written in the same places (rewrite rule 5).

pub mod actions;
pub mod check;
pub mod ci;
pub mod config;
pub mod conversations;
pub mod epic;
pub mod git;
pub mod gitlab;
pub mod herdr;
pub mod json;
pub mod local;
pub mod lock;
pub mod main_update;
pub mod model;
pub mod permissions;
pub mod reader;
pub mod record;
pub mod runs;
pub mod said;
pub mod setups;
pub mod stack;
pub mod time;
pub mod try_check;
pub mod wait;
