# Crunch - Output Compression Proxy (Codex CLI)

**Usage**: Token-optimized CLI proxy for shell commands.

## Rule

Always prefix shell commands with `crunch`.

Examples:

```bash
crunch git status
crunch cargo test
crunch npm run build
crunch pytest -q
```

## Meta Commands

```bash
crunch discover          # Missed savings analysis
crunch proxy <cmd>       # Run raw command without filtering
```

## Verification

```bash
crunch --version
which crunch
```
