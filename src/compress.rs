//! Central output compressor: `(command, raw output) -> compressed`.
//!
//! This is the shared chokepoint used by the PostToolUse hook (which receives a
//! command's already-captured output and may replace it with a smaller version)
//! and the natural home for two cross-cutting behaviors:
//!
//!   * **auto-bypass** — outputs below a configurable size are passed through
//!     unchanged. Small outputs gain little from compression and can be mangled
//!     by it, so bypassing them means the agent never needs a manual
//!     `crunch proxy` for small commands.
//!   * **self-sufficient output** — if compression doesn't meaningfully shrink
//!     the output, we keep the raw text (no half-compressed noise, no spurious
//!     "full output" hint).
//!
//! It never re-executes the command — it only filters text that was already
//! produced — so it is pipe-safe and side-effect free. Per-tool filtering reuses
//! the existing pure filter functions in each `*_cmd` module.

use crate::discover::classify::{classify_command, Classification};

/// Result of compressing pre-captured output.
pub struct Compressed {
    /// The text to show in place of the raw output.
    pub output: String,
    /// True only when we actually dropped meaningful content (so the caller may
    /// add a pointer to the full log).
    pub truncated: bool,
}

/// Compress pre-captured `raw` output for `command`.
///
/// Returns `None` when the output should be shown verbatim — i.e. it is below
/// the auto-bypass threshold, the command has no crunch filter, or compression
/// would not meaningfully shrink it. Returning `None` is the signal to keep the
/// model-visible output exactly as the tool produced it.
pub fn compress_output(command: &str, raw: &str, exit_code: i32) -> Option<Compressed> {
    // P3 auto-bypass: tiny outputs are shown as-is.
    let bypass_under = crate::config::cached_config()
        .display
        .auto_bypass_under_bytes;
    if raw.len() < bypass_under {
        return None;
    }

    let tool = match classify_command(command) {
        Classification::Supported {
            crunch_equivalent, ..
        } => crunch_equivalent,
        _ => return None,
    };

    let filtered = filter_for_tool(tool, raw, exit_code)?;

    // Self-sufficient: only treat it as a compression if we saved a meaningful
    // amount. Otherwise keep the raw output (avoids half-compressed noise).
    if !meaningfully_smaller(&filtered, raw) {
        return None;
    }

    let truncated = filtered.len() < raw.len();
    Some(Compressed {
        output: filtered,
        truncated,
    })
}

/// A filtered output "counts" as compression only if it saved more than a small
/// fixed margin over the raw output. Prevents emitting a near-identical (or
/// larger) "compressed" blob plus a pointer for no real benefit.
fn meaningfully_smaller(filtered: &str, raw: &str) -> bool {
    // Require at least 15% reduction AND at least 64 bytes saved.
    filtered.len() + 64 < raw.len() && (filtered.len() as f64) < (raw.len() as f64) * 0.85
}

/// Map a `crunch <tool>` equivalent to its pure, pre-captured-output filter.
///
/// Only tools whose filters operate on the tool's *natural* output (not a
/// crunch-controlled format like an injected `--output-format json`) are wired
/// here, since in the PostToolUse path the command ran exactly as the model
/// wrote it. More tools are wired incrementally via thin `pub` filter wrappers
/// in their `*_cmd` modules.
fn filter_for_tool(tool: &str, raw: &str, _exit_code: i32) -> Option<String> {
    match tool {
        "crunch mypy" => Some(crate::mypy_cmd::filter_mypy_output(raw)),
        "crunch prettier" => Some(crate::prettier_cmd::filter_prettier_output(raw)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a raw blob of a given byte size made of distinct lines.
    fn blob(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("line {i} some content here for length"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_auto_bypass_small_output_passes_through() {
        // A small mypy output is below the bypass threshold → None (show raw).
        let raw = "error.py:1: error: boom\n";
        assert!(raw.len() < 512);
        assert!(compress_output("mypy error.py", raw, 1).is_none());
    }

    #[test]
    fn test_unsupported_command_passes_through() {
        let raw = blob(100); // large, but not a crunch-supported tool
        assert!(raw.len() >= 512);
        assert!(compress_output("some-unknown-tool --flag", &raw, 0).is_none());
    }

    #[test]
    fn test_ignored_command_passes_through() {
        let raw = blob(100);
        assert!(compress_output("cd /tmp", &raw, 0).is_none());
    }

    #[test]
    fn test_meaningfully_smaller_margin() {
        let raw = blob(100);
        assert!(meaningfully_smaller("tiny", &raw));
        assert!(!meaningfully_smaller(&raw, &raw)); // identical → not smaller
                                                    // 90% size → not enough reduction
        let near = &raw[..(raw.len() as f64 * 0.9) as usize];
        assert!(!meaningfully_smaller(near, &raw));
    }
}
