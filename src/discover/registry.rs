// registry.rs — public facade that re-exports from focused submodules.
// All logic lives in classify.rs, rewrite.rs, and env_wrap.rs.

pub use super::classify::{
    category_avg_tokens, classify_command, has_crunch_disabled_prefix, split_command_chain,
    strip_disabled_prefix, Classification,
};
pub use super::rewrite::rewrite_command;
