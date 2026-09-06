import type {
  PromptAttachmentPart,
  RuntimeAttachment,
  RuntimeSession,
} from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import {
  promptSubmissionTranscriptMetadata,
  type PromptSubmissionResult,
} from "./prompt-runtime-api.js"
import {
  formatPromptSubmissionBody,
  promptSubmissionFailureTransition,
  promptSubmissionSuccessTransition,
  promptSubmissionAttachmentsToParts,
  parsePromptAgentAliasRoute,
  resolvePromptSubmissionTargetAgentId,
} from "@chariox/kernel-client/prompt-submission"
import type { TranscriptPromptMetadata } from "@chariox/kernel-client/transcript-entry-state"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"

export type NormalPromptSubmitControllerDeps = {
  getPendingAttachments: () => readonly PendingPromptAttachment[]
  waitForPendingAgentFocusTransition: () => Promise<void>
  getFocusedAgentId: () => string | null
  getSession: () => RuntimeSession
  hasAgent: (agentId: string) => boolean
  clearActiveToolLabels: () => void
  setProviderActivityLabel: (label: string | null) => void
  setActiveStatusLabel: (label: string | null) => void
  getAttachment: () => RuntimeAttachment | null
  getSessionId: () => string
  clearPromptText: () => void
  shouldInlineLocalFiles: () => boolean
  preparePromptAttachmentsForSubmit: (
    attachments: PromptAttachmentPart[],
    options: { inlineLocalFiles: boolean },
  ) => Promise<PromptAttachmentPart[]>
  beginSubmittedPromptUi: (rawPrompt: string) => SubmittedPromptUiSnapshot
  renderPromptTranscript: (prompt: string) => string
  appendUserPrompt: (text: string, agentId?: string | null, metadata?: TranscriptPromptMetadata) => void
  submitPrompt: (
    attachmentId: string,
    targetAgentId: string | null,
    prompt: string,
    attachments: PromptAttachmentPart[],
  ) => Promise<PromptSubmissionResult>
  applySessionState: (session: RuntimeSession) => void
  setStreamingAgentId: (agentId: string | null) => void
  setWorking: (working: boolean) => void
  updateSessionChrome: () => void
  setStatusLine: (line: string) => void
  recordPromptAreaHistoryEntry: (sessionId: string, rawPrompt: string) => void
  restoreFailedPromptUi: (snapshot: SubmittedPromptUiSnapshot | null | undefined) => boolean
  getSubmittingAgentId: () => string | null
  clearAgentBusy: (agentId: string | null | undefined) => void
  setSubmittingAgentId: (agentId: string | null) => void
  setSubmitting: (submitting: boolean) => void
  setFatalError: (message: string) => void
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  logInfo?: (message: string, fields: Record<string, unknown>) => void
  logError?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type NormalPromptSubmitController = {
  submit(rawPrompt: string, targetAgentIdOverride?: string | null): Promise<void>
}

export function createNormalPromptSubmitController(
  deps: NormalPromptSubmitControllerDeps,
): NormalPromptSubmitController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  return {
    async submit(rawPrompt, targetAgentIdOverride) {
      const prompt = formatPromptSubmissionBody(rawPrompt)
      const aliasRoute = parsePromptAgentAliasRoute(prompt)
      const renderedPrompt = aliasRoute?.prompt ?? prompt
      const rawAttachments = promptSubmissionAttachmentsToParts(deps.getPendingAttachments())
      let submissionUi: SubmittedPromptUiSnapshot | null = null
      try {
        await deps.waitForPendingAgentFocusTransition()
        const requestedTargetAgentId = targetAgentIdOverride ?? deps.getFocusedAgentId()
        const targetAgentId = resolvePromptSubmissionTargetAgentId({
          requestedTargetAgentId,
          hasAgent: deps.hasAgent,
        })
        deps.logInfo?.("submitting prompt", {
          chars: prompt.length,
          attachments: rawAttachments.length,
        })
        deps.clearActiveToolLabels()
        deps.setProviderActivityLabel(null)
        deps.setActiveStatusLabel(null)
        const attachment = deps.getAttachment()
        if (!attachment) {
          deps.flashFooter("No session attached.", "error")
          deps.clearPromptText()
          return
        }
        const attachments = await deps.preparePromptAttachmentsForSubmit(rawAttachments, {
          inlineLocalFiles: deps.shouldInlineLocalFiles(),
        })
        submissionUi = deps.beginSubmittedPromptUi(rawPrompt)
        const submission = await deps.submitPrompt(attachment.id, targetAgentId, prompt, attachments)
        const payload = submission.payload
        const submittedTargetAgentId = submission.targetAgentId ?? targetAgentId
        deps.applySessionState(payload.session)
        const outcomeName = submission.outcomeName
        const transition = promptSubmissionSuccessTransition({
          session: payload.session,
          outcomeName,
          submittedTargetAgentId,
        })
        if (transition.shouldAppendUserPrompt) {
          deps.appendUserPrompt(
            deps.renderPromptTranscript(renderedPrompt),
            submittedTargetAgentId,
            promptSubmissionTranscriptMetadata(payload, submittedTargetAgentId),
          )
        }
        deps.setStreamingAgentId(transition.streamingAgentId)
        deps.setWorking(transition.working)
        deps.updateSessionChrome()
        deps.logInfo?.("prompt submitted", {
          outcome: outcomeName,
          active_prompt_id: transition.activePromptId,
          queued_prompts: transition.queuedPromptCount,
        })
        deps.setStatusLine(transition.statusLine)
        deps.updateSessionChrome()
        deps.recordPromptAreaHistoryEntry(deps.getSessionId(), rawPrompt)
      } catch (error) {
        const message = formatError(error)
        deps.logError?.("prompt submission failed", {
          error: message,
        })
        deps.restoreFailedPromptUi(submissionUi)
        const transition = promptSubmissionFailureTransition({
          session: deps.getSession(),
          submittingAgentId: deps.getSubmittingAgentId(),
        })
        deps.clearAgentBusy(transition.clearBusyAgentId)
        deps.setSubmittingAgentId(transition.submittingAgentId)
        deps.setSubmitting(transition.submitting)
        deps.setStreamingAgentId(transition.streamingAgentId)
        deps.setWorking(transition.working)
        deps.setFatalError(message)
        deps.flashFooter(message, "error")
        deps.updateSessionChrome()
      }
    },
  }
}
