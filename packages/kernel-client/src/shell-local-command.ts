import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"

export function executeShellLocalCommand(parsed: ParsedShellCommand, context: ShellContext): ShellCommandResult {
  const [first, second] = parsed.args
  switch (parsed.command) {
    case "help":
      return {
        ok: true,
        message: [
          "chariox-shell commands:",
          "session list|status|new|attach|use|members|invites|invite|join|revoke-invite|mode|permissions",
          "kernel health|status|remote-runtime|runtime|debug-bundle [label]|delete",
          "agent list|spawn [--count <n>]|focus|inspect|cycle|mode|permissions|substitute",
          "client invite create|join|list|record|revoke",
          "machine invite create|join|list|kernels|approve|rename|revoke",
          "slice list|create|status|doctor|logs|audit|state|save-state|backup|reset-state|start|stop|delete|auth import|auth remove|auth login|screen",
          "  slice auth import copies a selected provider account into the slice; auth login starts provider login inside the slice; auth remove purges the selected slice-local account",
          "relay status",
          "config show|path|keys|schema|set|unset|workspace-live-sync off|managed|tracked",
          "credential list|show|register|upsert-json|set|delete",
          "mcp list|show|install|update|uninstall|import|grant|revoke|grants",
          "skill list|show|install|update|uninstall|import|grant|revoke|grants",
          "extension import providers|grant|revoke|grants|sync-status|sync-retry|audit",
          "workspace sync status|doctor|targets|conflicts|ignore|audit|off|managed|tracked|default|link",
          "workspace link create|list|show|attach|detach",
          "workflow list|new|show|run|runs|cancel|resume|node|edge|endpoint",
          "recall search|semantic-search",
          "prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]",
          "provider status|login|setup-token|logout|reauth|processes [provider]|processes teardown <provider>",
          "stop",
          "context",
          "pwd",
          "set provider|model|effort <value>",
          "use session|agent|workflow <ref>",
          "vars",
          "unset <name>",
          "exit",
        ].join("\n"),
      }
    case "pwd":
      return { ok: true, message: context.worktree }
    case "set": {
      if (first !== "provider" && first !== "model" && first !== "effort") {
        return { ok: false, message: "usage: set provider|model|effort <value>" }
      }
      if (!second) {
        return { ok: false, message: `usage: set ${first} <value>` }
      }
      return {
        ok: true,
        message: `${first} = ${second}`,
        contextUpdates: { [first]: second },
      }
    }
    case "use": {
      if (first !== "session" && first !== "agent" && first !== "workflow") {
        return { ok: false, message: "usage: use session|agent|workflow <ref>" }
      }
      if (!second) {
        return { ok: false, message: `usage: use ${first} <ref>` }
      }
      const key = first === "session" ? "sessionId" : first === "agent" ? "agentId" : "workflowId"
      return {
        ok: true,
        message: `current ${first} = ${second}`,
        contextUpdates: { [key]: second },
      }
    }
    case "vars": {
      const entries = Object.entries(context.variables)
      return {
        ok: true,
        message: entries.length === 0
          ? "no variables bound"
          : entries.map(([name, value]) => `$${name} = ${value}`).join("\n"),
      }
    }
    case "unset": {
      if (!first) {
        return { ok: false, message: "usage: unset <name>" }
      }
      const nextVariables = { ...context.variables }
      delete nextVariables[first]
      return {
        ok: true,
        message: `unset $${first}`,
        variableRemovals: [first],
        data: { variables: nextVariables },
      }
    }
    case "exit":
    case "quit":
      return { ok: true, message: "exit", data: { exit: true } }
    case "source":
    case "run":
      return { ok: false, message: "script execution is not implemented yet" }
    default:
      return { ok: false, message: `${parsed.command ?? "command"} is not implemented in chariox-shell yet` }
  }
}
