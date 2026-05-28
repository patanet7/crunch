# Crunch - Output Compression Proxy

**Usage**: Token-optimized CLI proxy (60-90% savings on dev operations)

## Meta Commands (always use crunch directly)

```bash
crunch discover          # Analyze Claude Code history for missed opportunities
```

## Installation Verification

```bash
crunch --version         # Should show: crunch X.Y.Z
which crunch             # Verify correct binary
```

## Hook-Based Usage

All other commands are automatically rewritten by the Claude Code hook.
Example: `git status` → `crunch git status` (transparent, 0 tokens overhead)

Refer to CLAUDE.md for full command reference.

## CRITICAL: Use Tee Logs — Never Re-run for More Detail

Crunch saves the **complete unfiltered output** to a log file before compressing. When command output includes a hint like:
```
[full output: /tmp/crunch/myproject/cargo_test-all-20260410-143012.log]
```

You MUST use the Read tool on that log path to get full details. **Do NOT re-run the command.** The log already contains everything the raw command would produce.

Rules:
1. If you need more detail than the compressed summary shows → **Read the log file**
2. If a test failed and you want the full traceback → **Read the log file**
3. If build errors were truncated → **Read the log file**
4. NEVER re-execute a command just to see uncompressed output — this wastes time and tokens
5. The only reason to re-run a command is if you changed code and need to verify the fix
