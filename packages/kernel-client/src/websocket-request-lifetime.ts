import { setTimeout as sleep } from "node:timers/promises"
import { LocalIpcError } from "./local-ipc-error.js"

// Explicit client shutdown retires outstanding transport work, not requests
// already accepted by the kernel. Later explicit sends may use a new lifetime.
export class KernelRequestLifetime {
  private controller = new AbortController()

  capture(): AbortSignal {
    return this.controller.signal
  }

  retire(message: string): void {
    const retired = this.controller
    this.controller = new AbortController()
    retired.abort(new LocalIpcError("kernel websocket", message, "client_closed", false))
  }
}

export async function waitForKernelRequestReplay(milliseconds: number, signal: AbortSignal): Promise<void> {
  signal.throwIfAborted()
  try {
    await sleep(milliseconds, undefined, { signal })
  } catch (error) {
    signal.throwIfAborted()
    throw error
  }
}
