# Crunch - Output Compression Proxy

**Usage**: Token-optimized CLI proxy (60-90% savings on dev operations)

## Meta Commands (always use crunch directly)

```bash
crunch discover          # Analyze Claude Code history for missed opportunities
crunch proxy <cmd>       # Execute raw command without filtering (for debugging)
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
