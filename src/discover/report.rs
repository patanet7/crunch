use serde::Serialize;

/// Crunch support status for a command.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
pub enum CrunchStatus {
    /// Dedicated handler with filtering (e.g., git status → git.rs:run_status())
    Existing,
    /// Works via external_subcommand passthrough, no filtering (e.g., cargo fmt → Other)
    Passthrough,
    /// Crunch doesn't handle this command at all
    NotSupported,
}

impl CrunchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CrunchStatus::Existing => "existing",
            CrunchStatus::Passthrough => "passthrough",
            CrunchStatus::NotSupported => "not-supported",
        }
    }
}

/// A supported command that Crunch already handles.
#[derive(Debug, Serialize)]
pub struct SupportedEntry {
    pub command: String,
    pub count: usize,
    pub crunch_equivalent: &'static str,
    pub category: &'static str,
    pub estimated_savings_tokens: usize,
    pub estimated_savings_pct: f64,
    pub crunch_status: CrunchStatus,
}

/// An unsupported command not yet handled by Crunch.
#[derive(Debug, Serialize)]
pub struct UnsupportedEntry {
    pub base_command: String,
    pub count: usize,
    pub example: String,
}

/// Full discover report.
#[derive(Debug, Serialize)]
pub struct DiscoverReport {
    pub sessions_scanned: usize,
    pub total_commands: usize,
    pub already_crunch: usize,
    pub since_days: u64,
    pub supported: Vec<SupportedEntry>,
    pub unsupported: Vec<UnsupportedEntry>,
    pub parse_errors: usize,
    pub crunch_disabled_count: usize,
    pub crunch_disabled_examples: Vec<String>,
}

impl DiscoverReport {
    pub fn total_saveable_tokens(&self) -> usize {
        self.supported
            .iter()
            .map(|s| s.estimated_savings_tokens)
            .sum()
    }

    pub fn total_supported_count(&self) -> usize {
        self.supported.iter().map(|s| s.count).sum()
    }
}

/// Format report as text.
pub fn format_text(report: &DiscoverReport, limit: usize, verbose: bool) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str("Crunch Discover -- Savings Opportunities\n");
    out.push_str(&"=".repeat(52));
    out.push('\n');
    out.push_str(&format!(
        "Scanned: {} sessions (last {} days), {} Bash commands\n",
        report.sessions_scanned, report.since_days, report.total_commands
    ));
    out.push_str(&format!(
        "Already using Crunch: {} commands ({}%)\n",
        report.already_crunch,
        if report.total_commands > 0 {
            report.already_crunch * 100 / report.total_commands
        } else {
            0
        }
    ));

    if report.supported.is_empty() && report.unsupported.is_empty() {
        out.push_str("\nNo missed savings found. Crunch usage looks good!\n");
        return out;
    }

    // Missed savings
    if !report.supported.is_empty() {
        out.push_str("\nMISSED SAVINGS -- Commands Crunch already handles\n");
        out.push_str(&"-".repeat(72));
        out.push('\n');
        out.push_str(&format!(
            "{:<24} {:>5}    {:<18} {:<13} {:>12}\n",
            "Command", "Count", "Crunch Equiv", "Status", "Est. Savings"
        ));

        for entry in report.supported.iter().take(limit) {
            out.push_str(&format!(
                "{:<24} {:>5}    {:<18} {:<13} ~{}\n",
                truncate_str(&entry.command, 23),
                entry.count,
                entry.crunch_equivalent,
                entry.crunch_status.as_str(),
                format_tokens(entry.estimated_savings_tokens),
            ));
        }

        out.push_str(&"-".repeat(72));
        out.push('\n');
        out.push_str(&format!(
            "Total: {} commands -> ~{} saveable\n",
            report.total_supported_count(),
            format_tokens(report.total_saveable_tokens()),
        ));
    }

    // Unhandled
    if !report.unsupported.is_empty() {
        out.push_str("\nTOP UNHANDLED COMMANDS -- open an issue?\n");
        out.push_str(&"-".repeat(52));
        out.push('\n');
        out.push_str(&format!(
            "{:<24} {:>5}    {}\n",
            "Command", "Count", "Example"
        ));

        for entry in report.unsupported.iter().take(limit) {
            out.push_str(&format!(
                "{:<24} {:>5}    {}\n",
                truncate_str(&entry.base_command, 23),
                entry.count,
                truncate_str(&entry.example, 40),
            ));
        }

        out.push_str(&"-".repeat(52));
        out.push('\n');
        out.push_str("\n");
    }

    // CRUNCH_DISABLED bypass warning
    if report.crunch_disabled_count > 0 {
        out.push_str(&format!(
            "\nCRUNCH_DISABLED BYPASS -- {} commands ran without filtering\n",
            report.crunch_disabled_count
        ));
        out.push_str(&"-".repeat(72));
        out.push('\n');
        out.push_str("These commands used CRUNCH_DISABLED=1 unnecessarily:\n");
        if !report.crunch_disabled_examples.is_empty() {
            out.push_str(&format!(
                "  {}\n",
                report.crunch_disabled_examples.join(", ")
            ));
        }
        out.push_str("-> Remove CRUNCH_DISABLED=1 to recover token savings\n");
    }

    out.push_str("\n~estimated from tool_result output sizes\n");

    // Cursor note: check if Cursor hooks are installed
    if let Some(home) = dirs::home_dir() {
        let cursor_hook = home.join(".cursor").join("hooks").join("crunch-rewrite.sh");
        if cursor_hook.exists() {
            out.push_str("\nNote: Cursor sessions are tracked via `crunch gain` (discover scans Claude Code only)\n");
        }
    }

    if verbose && report.parse_errors > 0 {
        out.push_str(&format!("Parse errors skipped: {}\n", report.parse_errors));
    }

    out
}

/// Format report as JSON.
pub fn format_json(report: &DiscoverReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
}

fn format_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M tokens", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K tokens", tokens as f64 / 1_000.0)
    } else {
        format!("{} tokens", tokens)
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // UTF-8 safe truncation: collect chars up to max-2, then add ".."
        let truncated: String = s
            .char_indices()
            .take_while(|(i, _)| *i < max.saturating_sub(2))
            .map(|(_, c)| c)
            .collect();
        format!("{}..", truncated)
    }
}
