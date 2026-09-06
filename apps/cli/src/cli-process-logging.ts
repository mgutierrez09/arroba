import {
  createProcessLogger,
  type CharioxLogger,
  type LogFields,
} from "./logging.js"
import { describeCliError } from "./runtime.js"

export type CreateCliProcessLogger = (processKind: string, component?: string) => CharioxLogger

const SECRET_VALUE_OPTIONS = new Set([
  "--relay-token",
  "--terminal-pairing-link",
  "--pairing-link",
])

export function redactCliStartupArgs(argv: readonly string[]): string[] {
  let redactNext = false
  return argv.map((arg) => {
    if (redactNext) {
      redactNext = false
      return "[redacted]"
    }
    if (SECRET_VALUE_OPTIONS.has(arg)) {
      redactNext = true
      return arg
    }
    const inlineSecretOption = [...SECRET_VALUE_OPTIONS]
      .find((option) => arg.startsWith(`${option}=`))
    if (inlineSecretOption) {
      return `${inlineSecretOption}=[redacted]`
    }
    if (arg.startsWith("chariox-terminal-pair-v1.")) {
      return "[redacted-terminal-pairing-link]"
    }
    return arg
  })
}

export function createCliProcessLoggerRegistry(options: {
  createLogger?: CreateCliProcessLogger
} = {}) {
  const createLogger = options.createLogger ?? createProcessLogger
  let processLogger: CharioxLogger | null = null

  return {
    initialize(processKind: string) {
      processLogger = createLogger(processKind)
      return processLogger
    },

    getLogger(component: string, fields: LogFields = {}) {
      return processLogger?.child(component, fields) ?? null
    },
  }
}

export function formatCliError(error: unknown): string {
  return describeCliError(error)
}
