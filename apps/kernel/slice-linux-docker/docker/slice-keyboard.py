#!/usr/bin/env python3
"""Physical text input using the pinned Selkies XTEST keyboard implementation."""

import logging
import signal
import sys
import time

logging.disable(logging.CRITICAL)

from selkies import Xlib
from selkies.Xlib import display
from selkies.Xlib.ext import xtest
# Internal API is intentionally tied to selkies.lock.json revision
# 3f87241fcd6abc44e205b22f6596e78ef4946670. Any pin upgrade must rerun the
# physical keyboard X11 drill, including Unicode recycling and cancellation.
from selkies.input_handler import (
    _XTestKeyboard,
    character_to_layout_keysym,
    universal_text_keysym,
)


def type_text(text):
    connection = display.Display()
    keyboard = _XTestKeyboard(connection)
    lifted = []
    active_keysym = None
    try:
        keysyms = []
        for character in text:
            # Some layouts carry Linefeed, which Chromium accepts but GTK
            # editors ignore. Text line breaks need the universal Return
            # binding even when a layout advertises the control character.
            if character in ("\n", "\r", "\t"):
                keysym = universal_text_keysym(character)
            else:
                keysym = character_to_layout_keysym(character)
                if not keyboard.layout_carries(keysym):
                    keysym = universal_text_keysym(character)
            if keysym is None:
                raise ValueError("unsupported text character")
            keysyms.append(keysym)

        # Reuse Selkies' persistent overlay, including its bounded recycling
        # when a string contains more distinct symbols than the spare pool.
        keyboard.prebind(keysyms)
        down = connection.query_keymap()
        modifiers = {code for row in connection.get_modifier_mapping() for code in row if code}
        lifted = [code for code in modifiers if down[code // 8] & (1 << (code % 8))]
        for code in lifted:
            xtest.fake_input(connection, Xlib.X.KeyRelease, code)
        connection.sync()

        for keysym in keysyms:
            active_keysym = keysym
            keyboard.press(keysym)
            keyboard.release(keysym)
            active_keysym = None
            # Pace on this process, not in the X server's request queue. Killing
            # the kernel-owned process group must stop future physical events.
            connection.sync()
            time.sleep(0.04)
    finally:
        # A second termination signal must not interrupt modifier restoration.
        # The caller retains SIGKILL as its bounded last-resort cleanup.
        for signum in (signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, signal.SIG_IGN)
        if active_keysym is not None:
            keyboard.release(active_keysym)
        keyboard.release_group_lock()
        for code in lifted:
            xtest.fake_input(connection, Xlib.X.KeyPress, code)
        connection.sync()
        connection.close()


def reset_input():
    connection = display.Display()
    try:
        down = connection.query_keymap()
        for code in range(8, 256):
            if down[code // 8] & (1 << (code % 8)):
                xtest.fake_input(connection, Xlib.X.KeyRelease, code)
        for button in range(1, 6):
            xtest.fake_input(connection, Xlib.X.ButtonRelease, button)
        connection.sync()
    finally:
        connection.close()


if __name__ == "__main__":
    def terminate(signum, _frame):
        raise SystemExit(128 + signum)

    for signum in (signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, terminate)
    try:
        if sys.argv[1:] == ["reset"]:
            reset_input()
        elif not sys.argv[1:]:
            type_text(sys.stdin.buffer.read().decode("utf-8", errors="strict"))
        else:
            raise ValueError("unsupported keyboard operation")
    except Exception:
        # Neither typed text nor upstream exceptions belong in helper output.
        print("physical keyboard text input failed", file=sys.stderr)
        sys.exit(1)
