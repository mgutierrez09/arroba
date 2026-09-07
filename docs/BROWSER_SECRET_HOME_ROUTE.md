# Browser secret insertion uses the home Room route

The controlled official-provider onboarding drill reproduced a routing failure:
`paste_secret_to_slice` ran against a provisioned worker-local controller and
was rejected with `browser_controller_scope_denied`. Ordinary Browser actions
were already forwarded to the home Room. Retrying the rejected local route
left that worker-local environment in a failed lifecycle state.

Password insertion now uses the same `ForwardRoomBrowserRuntimeTool` request
and home authorizer as ordinary Browser actions. The shared allowlist accepts
only the existing browser-secret tool among credential tools, including its
existing aliases. Other credential tools remain on their own established path.
No serialized request, response or protocol version changes are required.

The home validates the relay sender, lease, bound worker, reserved Room slice
and agent membership before dispatch. It then validates the opaque field,
masked target, expected host/URL and actual target-document credential scope.
Only then may it request a home vault unlock and resolve the secret. The fill
reloads credential metadata and vault configuration after that interaction,
so deletion, revocation or narrowed scope during the wait is not bypassed by
a pre-interaction authorization snapshot. The fill
uses the existing attributed Browser locator action, including target-document
revalidation at execution. The provider receives metadata, not the password.
The worker does not receive a credential-resolution response for this route.

Forwarding allows 345 seconds for the existing 300-second vault interaction,
Browser work and relay response, without reducing a longer configured timeout.
The legacy non-controller local slice path is unchanged. Neither route enables
direct worker access to a provisioned controller.

## Validation

The original failing command is the opt-in local Room onboarding drill with
`CHARIOX_ROOM_DRILL_FOCUS=real-provider`,
`CHARIOX_ROOM_DRILL_PROVIDER_MODE=browser` and
`CHARIOX_ROOM_DRILL_OFFICE_SCENARIO=onboarding`.

Routing/alias and timeout unit tests accompany the change. Full acceptance
requires freshly built matching home/worker binaries and image, the original
official-provider onboarding flow, local and remote TUI receipts, and scoped
credential rejection checks. Source checks alone do not prove that acceptance.
Run local validation before managed-machine and benchmark work.
