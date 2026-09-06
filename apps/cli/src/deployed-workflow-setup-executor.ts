import {
  checkpointDeploymentSetup,
  DeploymentSetupRequestError,
  getDeploymentSetup,
  type DeploymentSetup,
  type DeploymentSetupCheckpoint,
} from "./deployed-workflow-setup-api.js"
import type { RelayCloudProfile } from "./preferences.js"

export type DeploymentSetupExecutionOutcome =
  | { readonly kind: "completed"; readonly setup: DeploymentSetup }
  | { readonly kind: "awaiting_credentials"; readonly setup: DeploymentSetup }
  | { readonly kind: "waiting_for_runtime"; readonly setup: DeploymentSetup }
  | { readonly kind: "activation_requested"; readonly setup: DeploymentSetup }
  | { readonly kind: "blocked"; readonly setup: DeploymentSetup }
  | { readonly kind: "abandoned"; readonly setup: DeploymentSetup }

export interface DeploymentSetupExecutorOperations {
  readonly publishSource: (setup: DeploymentSetup) => Promise<{
    readonly publicationId: string
    readonly publicationDigest: string
  }>
  readonly exportPackage: (setup: DeploymentSetup) => Promise<{
    readonly packageId: string
    readonly packageDigest: string
  }>
  readonly resolveProject: (setup: DeploymentSetup) => Promise<{
    readonly projectId: string
    readonly environmentId: string
  }>
  readonly verifyRelease: (setup: DeploymentSetup) => Promise<{ readonly releaseId: string }>
  readonly credentialsReady: (setup: DeploymentSetup) => Promise<boolean>
  readonly bindRuntime: (setup: DeploymentSetup) => Promise<{
    readonly operationalDeploymentId: string
    readonly state: "running" | "waiting_for_relay"
  }>
  readonly activateHosted: (setup: DeploymentSetup) => Promise<{
    readonly promotionId: string
    readonly operationalDeploymentId?: string | null
  }>
  readonly stageChanged?: (setup: DeploymentSetup) => void
}

export async function executeDeploymentSetup(
  profile: RelayCloudProfile,
  setupId: string,
  operations: DeploymentSetupExecutorOperations,
): Promise<DeploymentSetupExecutionOutcome> {
  let setup = (await getDeploymentSetup(profile, setupId)).setup
  for (let transition = 0; transition < 24; transition += 1) {
    operations.stageChanged?.(setup)
    if (setup.status === "blocked") return { kind: "blocked", setup }
    if (setup.status === "abandoned") return { kind: "abandoned", setup }
    if (setup.status === "completed" || setup.stage === "complete") {
      return { kind: "completed", setup }
    }
    if (setup.status !== "active") {
      throw new Error(`deployment setup ${setup.id} has unsupported status ${setup.status}`)
    }

    switch (setup.stage) {
      case "source": {
        const publication = await operations.publishSource(setup)
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.publication, {
          kind: "source_published",
          publicationId: publication.publicationId,
          publicationDigest: publication.publicationDigest,
        })
        break
      }
      case "package": {
        const exported = await operations.exportPackage(setup)
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.package, {
          kind: "package_exported",
          packageId: exported.packageId,
          packageDigest: exported.packageDigest,
        })
        break
      }
      case "project": {
        const project = await operations.resolveProject(setup)
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.project, {
          kind: "project_resolved",
          projectId: project.projectId,
          environmentId: project.environmentId,
        })
        break
      }
      case "release": {
        const release = await operations.verifyRelease(setup)
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.release, {
          kind: "release_verified",
          releaseId: release.releaseId,
        })
        break
      }
      case "credentials": {
        if (!await operations.credentialsReady(setup)) {
          return { kind: "awaiting_credentials", setup }
        }
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.credentials, {
          kind: "credentials_ready",
        })
        break
      }
      case "runtime": {
        const binding = await operations.bindRuntime(setup)
        if (binding.state !== "running") return { kind: "waiting_for_runtime", setup }
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.runtime, {
          kind: "runtime_bound",
          operationalDeploymentId: binding.operationalDeploymentId,
        })
        break
      }
      case "activation": {
        if (setup.promotionId) {
          return { kind: "activation_requested", setup }
        }
        const activation = await operations.activateHosted(setup)
        setup = await checkpointOrReload(profile, setup, setup.operationKeys.promotion, {
          kind: "activation_requested",
          promotionId: activation.promotionId,
          operationalDeploymentId: activation.operationalDeploymentId ?? null,
        })
        break
      }
    }
  }
  throw new Error(`deployment setup ${setupId} exceeded the transition limit`)
}

async function checkpointOrReload(
  profile: RelayCloudProfile,
  setup: DeploymentSetup,
  operationKey: string,
  checkpoint: DeploymentSetupCheckpoint,
): Promise<DeploymentSetup> {
  try {
    return (await checkpointDeploymentSetup(profile, {
      setupId: setup.id,
      expectedVersion: setup.version,
      operationKey,
      checkpoint,
    })).setup
  } catch (error) {
    if (!isSetupConflict(error)) throw error
    const current = (await getDeploymentSetup(profile, setup.id)).setup
    if (
      current.version === setup.version
      && current.stage === setup.stage
      && current.status === setup.status
    ) {
      throw error
    }
    return current
  }
}

function isSetupConflict(error: unknown): boolean {
  if (!(error instanceof DeploymentSetupRequestError) || error.status !== 409) return false
  const body = objectRecord(error.payload)
  return objectRecord(body?.error)?.code === "deployment_setup_conflict"
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}
