import type { Plugin } from "@opencode-ai/plugin"

// Crunch OpenCode plugin — rewrites commands to use crunch for token savings.
// Requires: crunch >= 0.23.0 in PATH.
//
// This is a thin delegating plugin: all rewrite logic lives in `crunch rewrite`,
// which is the single source of truth (src/discover/registry.rs).
// To add or change rewrite rules, edit the Rust registry — not this file.

export const CrunchOpenCodePlugin: Plugin = async ({ $ }) => {
  try {
    await $`which crunch`.quiet()
  } catch {
    console.warn("[crunch] crunch binary not found in PATH — plugin disabled")
    return {}
  }

  return {
    "tool.execute.before": async (input, output) => {
      const tool = String(input?.tool ?? "").toLowerCase()
      if (tool !== "bash" && tool !== "shell") return
      const args = output?.args
      if (!args || typeof args !== "object") return

      const command = (args as Record<string, unknown>).command
      if (typeof command !== "string" || !command) return

      try {
        const result = await $`crunch rewrite ${command}`.quiet().nothrow()
        const rewritten = String(result.stdout).trim()
        if (rewritten && rewritten !== command) {
          ;(args as Record<string, unknown>).command = rewritten
        }
      } catch {
        // crunch rewrite failed — pass through unchanged
      }
    },
  }
}
