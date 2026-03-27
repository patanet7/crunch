# Crunch - Output Compression Proxy (Windsurf)

**Usage**: Token-optimized CLI proxy for shell commands.

## Rule

Always prefix shell commands with `crunch` to minimize token consumption.

Examples:

```bash
crunch git status
crunch cargo test
crunch ls src/
crunch grep "pattern" src/
crunch find "*.rs" .
crunch docker ps
crunch gh pr list
```

## Meta Commands

```bash
crunch discover          # Find missed Crunch opportunities
crunch proxy <cmd>       # Run raw (no filtering, for debugging)
```

## Why

Crunch filters and compresses command output before it reaches the LLM context, saving 60-90% tokens on common operations. Always use `crunch <cmd>` instead of raw commands.
