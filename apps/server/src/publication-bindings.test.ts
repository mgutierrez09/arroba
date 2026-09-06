import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { resolvePublicationProviderModelBindings } from "./publication-bindings.js"
import type { WorkflowPublicationSnapshot } from "./publication-types.js"
import type { WorkflowPublicationDeploymentContract } from "@chariox/kernel-client/workflow-publication-deployment-contract"

test("publication bindings resolve the default model sentinel without prompting", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "codex",
      model: "default",
      effort: null,
    }],
  } as unknown as WorkflowPublicationSnapshot

  const resolved = await resolvePublicationProviderModelBindings(
    snapshot,
    "/tmp/chariox-publication-default-model-bindings-does-not-exist.json",
    {
      send: async <T>(): Promise<T> => ({
        ProviderCatalog: {
          catalog: {
            all: [{ id: "codex", models: { "gpt-5.6-sol": {} } }],
          },
        },
      }) as T,
    },
    { promptReplacement: false },
  )

  assert.equal(resolved.snapshot.agents?.[0]?.model, null)
  assert.equal(resolved.changed, false)
})

test("publication bindings reject a default model when the provider is unavailable", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "missing-provider",
      model: "default",
      effort: null,
    }],
  } as unknown as WorkflowPublicationSnapshot

  await assert.rejects(
    resolvePublicationProviderModelBindings(
      snapshot,
      "/tmp/chariox-publication-missing-provider-bindings-does-not-exist.json",
      {
        send: async <T>(): Promise<T> => ({
          ProviderCatalog: {
            catalog: {
              all: [{ id: "codex", models: { "gpt-5.6-sol": {} } }],
            },
          },
        }) as T,
      },
      { promptReplacement: false },
    ),
    /publication provider\/model is unavailable/,
  )
})

test("publication bindings keep OpenCode models provider-qualified after catalog validation", async () => {
  for (const capturedModel of ["gpt-5.2", "opencode/gpt-5.2"]) {
    const snapshot = {
      workflow: {
        id: "workflow-1",
        nodes: [{ id: "node-1", agent_id: "agent-1" }],
      },
      agents: [{
        id: "agent-1",
        provider: "opencode",
        model: capturedModel,
        effort: null,
      }],
    } as unknown as WorkflowPublicationSnapshot

    const resolved = await resolvePublicationProviderModelBindings(
      snapshot,
      `/tmp/chariox-publication-opencode-${capturedModel.replaceAll("/", "-")}-bindings-does-not-exist.json`,
      {
        send: async <T>(): Promise<T> => ({
          ProviderCatalog: {
            catalog: {
              all: [{ id: "opencode", models: { "gpt-5.2": {} } }],
            },
          },
        }) as T,
      },
      { promptReplacement: false },
    )

    assert.equal(resolved.snapshot.agents?.[0]?.model, "opencode/gpt-5.2")
    assert.equal(resolved.changed, false)
  }
})

test("publication bindings validate OpenCode Go models against the selected catalog provider", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "opencode",
      model: "opencode-go/deepseek-v4-flash",
      effort: "low",
    }],
  } as unknown as WorkflowPublicationSnapshot

  const resolved = await resolvePublicationProviderModelBindings(
    snapshot,
    "/tmp/chariox-publication-opencode-go-bindings-does-not-exist.json",
    {
      send: async <T>(): Promise<T> => ({
        ProviderCatalog: {
          catalog: {
            all: [
              { id: "opencode", models: { "deepseek-v4-flash": {} } },
              { id: "opencode-go", models: { "deepseek-v4-flash": {} } },
            ],
          },
        },
      }) as T,
    },
    { promptReplacement: false },
  )

  assert.equal(resolved.snapshot.agents?.[0]?.model, "opencode-go/deepseek-v4-flash")
  assert.equal(resolved.changed, false)
})

test("publication bindings do not validate OpenCode Go models against the Zen catalog", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "opencode",
      model: "opencode-go/deepseek-v4-flash",
      effort: "low",
    }],
  } as unknown as WorkflowPublicationSnapshot

  await assert.rejects(
    resolvePublicationProviderModelBindings(
      snapshot,
      "/tmp/chariox-publication-opencode-go-zen-only-bindings-does-not-exist.json",
      {
        send: async <T>(): Promise<T> => ({
          ProviderCatalog: {
            catalog: {
              all: [{ id: "opencode", models: { "deepseek-v4-flash": {} } }],
            },
          },
        }) as T,
      },
      { promptReplacement: false },
    ),
    /publication provider\/model is unavailable/,
  )
})

test("publication bindings validate the Claude family against runtime adapter catalogs", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "claude",
      model: "claude-sonnet-5",
      effort: null,
    }],
  } as unknown as WorkflowPublicationSnapshot

  const resolved = await resolvePublicationProviderModelBindings(
    snapshot,
    "/tmp/chariox-publication-claude-family-bindings-does-not-exist.json",
    {
      send: async <T>(): Promise<T> => ({
        ProviderCatalog: {
          catalog: {
            all: [{ id: "claude-headless", models: { "claude-sonnet-5": {} } }],
          },
        },
      }) as T,
    },
    { promptReplacement: false },
  )

  assert.equal(resolved.snapshot.agents?.[0]?.provider, "claude")
  assert.equal(resolved.snapshot.agents?.[0]?.model, "claude-sonnet-5")
  assert.equal(resolved.changed, false)
})

test("publication bindings apply a destination account profile without packaging source identity", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-account-binding-"))
  const bindingsPath = join(root, "bindings.local.json")
  const snapshot = {
    workflow: { id: "workflow-1", nodes: [{ id: "node-1", agent_id: "agent-1" }] },
    agents: [{ id: "agent-1", provider: "codex", model: "gpt-5.6-luna", effort: "low" }],
  } as unknown as WorkflowPublicationSnapshot
  try {
    await writeFile(bindingsPath, JSON.stringify({
      schema_version: 1,
      provider_model_overrides: [{
        agent_id: "agent-1",
        captured: { provider: "codex", model: "gpt-5.6-luna", effort: "low" },
        replacement: {
          provider: "codex",
          model: "gpt-5.6-luna",
          effort: "low",
          account_profile: "destination-profile-2",
        },
      }],
    }))
    const resolved = await resolvePublicationProviderModelBindings(snapshot, bindingsPath, {
      send: async <T>(): Promise<T> => ({
        ProviderCatalog: { catalog: { all: [{ id: "codex", models: { "gpt-5.6-luna": {} } }] } },
      }) as T,
    }, { promptReplacement: false })

    assert.equal(resolved.snapshot.agents?.[0]?.account_profile, "destination-profile-2")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication bindings never prompt or persist providers outside the immutable contract", async () => {
  const snapshot = {
    workflow: { id: "workflow-1", nodes: [{ id: "node-1", agent_id: "agent-1" }] },
    agents: [{ id: "agent-1", provider: "codex", model: "missing-model", effort: null }],
  } as unknown as WorkflowPublicationSnapshot
  let offeredProviders: string[] = []
  await assert.rejects(
    resolvePublicationProviderModelBindings(
      snapshot,
      "/tmp/chariox-publication-provider-policy-bindings-does-not-exist.json",
      {
        send: async <T>(): Promise<T> => ({
          ProviderCatalog: {
            catalog: { all: [
              { id: "codex", models: { "gpt-5.6": {} } },
              { id: "claude", models: { sonnet: {} } },
            ] },
          },
        }) as T,
      },
      {
        deploymentContract: providerPolicyContract(),
        promptReplacement: async ({ available }) => {
          offeredProviders = [...available.providers.keys()]
          return { provider: "claude", model: "sonnet", effort: null }
        },
      },
    ),
    /replacement is not permitted/,
  )
  assert.deepEqual(offeredProviders, ["codex"])
})

function providerPolicyContract(): WorkflowPublicationDeploymentContract {
  return {
    schema_version: 1,
    package_id: `sha256:${"a".repeat(64)}`,
    artifact: {
      content_digest: `sha256:${"a".repeat(64)}`,
      digest_algorithm: "sha256",
      digest_scope: "package_files_excluding_deployment_contract",
    },
    source: {
      publication_id: "publication-1",
      session_id: "session-1",
      workflow_id: "workflow-1",
      endpoint_id: "endpoint-1",
      creator_user_id: "user-1",
      captured_at_ms: 1,
    },
    compatibility: {
      package_version: 4,
      minimum_kernel_version: "0.1.0",
      minimum_local_daemon_protocol_version: 1,
    },
    routes: [],
    provider_requirements: [{ slot_id: "provider:codex", provider: "codex" }],
    credential_slots: [{ slot_id: "provider:codex", allowed_destination_ids: [] }],
    configuration: [{
      kind: "provider_profile",
      agent_id: "agent-1",
      allowed_providers: ["codex"],
      captured: { provider: "codex" },
    }],
    capabilities: {
      extensions: [],
      network: {
        policy_version: 1,
        default_action: "deny",
        destinations: [],
        provider_access: [{
          slot_id: "provider:codex",
          bundle_kind: "platform_managed",
          bundle_id: "codex-official-v1",
        }],
      },
    },
    resources: {},
    presentation: {},
    signatures: [],
  }
}
