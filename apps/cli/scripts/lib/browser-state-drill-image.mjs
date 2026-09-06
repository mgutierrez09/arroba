export function browserStateDrillImageConfig(env) {
  const image = env.M20_SLICE_IMAGE
  if (image === undefined) return ['build_image = "auto"']
  // A selected validation image must never trigger an unexpected Docker build
  // or replace the shared default tag. The provisioner checks its source hash.
  if (!image || image !== image.trim() || !/^[a-zA-Z0-9][a-zA-Z0-9._/:@-]*$/.test(image)) {
    throw new Error("M20_SLICE_IMAGE must be a nonempty Docker image reference")
  }
  return [`docker_image = ${JSON.stringify(image)}`, 'build_image = "never"']
}
