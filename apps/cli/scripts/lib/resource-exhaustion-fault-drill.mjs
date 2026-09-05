export const RESOURCE_EXHAUSTION_CASE_IDS = Object.freeze([
  "fault.resource-exhaustion",
  "cleanup.resources",
])

export const PROCESS_LIMIT_HEADROOM = "32"
export const PROCESS_LIMIT_SHELL = [
  'if [ "$(uname -s)" = "Linux" ]; then',
  'current_tasks="$(ps -eLo uid= | awk -v uid="$(id -u)" \'$1 == uid { count += 1 } END { print count + 0 }\')"',
  "else",
  'current_tasks="$(ps -u "$(id -u)" -o pid= | wc -l | tr -d \' \')"',
  "fi",
  'ulimit -u "$((current_tasks + $1))"',
].join("\n")

const PROBE_SCHEMA = "chariox.resource_exhaustion_probe.v1"
const EXPECTED_MODES = Object.freeze(["file-descriptor", "process"])
const EXPECTED_CODES = Object.freeze({
  "file-descriptor": new Set(["EMFILE", "ENFILE"]),
  process: new Set(["EAGAIN"]),
})

export function boundedEvidenceText(value, limit = 4_000) {
  const text = String(value ?? "").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "")
  return text.length <= limit ? text : text.slice(-limit)
}

export function parseResourceExhaustionProbes(outputs) {
  if (!Array.isArray(outputs) || outputs.length !== EXPECTED_MODES.length) {
    throw new Error("resource exhaustion drill must return one file-descriptor and one process probe")
  }
  const probes = new Map()
  for (const output of outputs) {
    let probe
    try {
      probe = JSON.parse(String(output).trim().split("\n").at(-1))
    } catch {
      throw new Error("resource exhaustion probe is not valid JSON")
    }
    const expectedKeys = ["cleanupComplete", "errorCode", "exhausted", "mode", "schema", "terminalLaneLive"]
    if (probe?.schema !== PROBE_SCHEMA || JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
      throw new Error("resource exhaustion probe fields do not match its schema")
    }
    if (!EXPECTED_MODES.includes(probe.mode) || probes.has(probe.mode)) {
      throw new Error(`resource exhaustion probe mode is invalid: ${probe.mode}`)
    }
    if (probe.exhausted !== true || !EXPECTED_CODES[probe.mode].has(probe.errorCode)) {
      throw new Error(`${probe.mode} exhaustion did not emit its actionable operating-system diagnostic`)
    }
    if (probe.terminalLaneLive !== true) {
      throw new Error(`${probe.mode} exhaustion did not preserve the terminal lane`)
    }
    if (probe.cleanupComplete !== true) {
      throw new Error(`${probe.mode} exhaustion did not clean up its owned resources`)
    }
    probes.set(probe.mode, probe)
  }
  for (const mode of EXPECTED_MODES) {
    if (!probes.has(mode)) throw new Error(`resource exhaustion drill is missing the ${mode} probe`)
  }
  return {
    schema: "chariox.resource_exhaustion_fault_result.v1",
    fileDescriptorLimitEnforced: true,
    processLimitEnforced: true,
    terminalLaneLive: true,
    actionableDiagnostics: Object.fromEntries(EXPECTED_MODES.map((mode) => [mode, probes.get(mode).errorCode])),
    cleanupComplete: true,
  }
}
