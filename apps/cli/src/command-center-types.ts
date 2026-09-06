export type CommandCenterItem = {
  id: string
  label: string
  description: string
  kind: "agent" | "command" | "group" | "provider" | "account" | "model" | "variant"
  value: string
  tone?: "warning" | "danger" | undefined
  searchAliases?: string[] | undefined
}
