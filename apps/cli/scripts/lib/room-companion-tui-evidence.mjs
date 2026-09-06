export async function verifyRoomCompanionTuis(input, actions, evidence) {
  await input.activityController.synchronize()
  const observed = []
  for (const action of actions) {
    const receipt = { actionId: action.action_id, sequence: action.sequence }
    await Promise.all(["local", "remote"].map(async side => {
      receipt[side] = evidence?.find(side, action)
      if (!receipt[side]) {
        await (side === "local"
          ? input.waitForLocalActionNotice(input.localNoticeIds, action)
          : input.waitForRemoteActionNotice(input.remoteNoticeIds, action))
        receipt[side] = { source: "final-snapshot", observedAt: new Date().toISOString() }
      }
    }))
    observed.push(receipt)
  }
  return { ...evidence?.summary(), actions: observed }
}
