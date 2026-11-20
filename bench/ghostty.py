"""Drive Ghostty through AppleScript (osascript). Commands and properties come from
/Applications/Ghostty.app/Contents/Resources/Ghostty.sdef."""

import shlex
import subprocess
from pathlib import Path
from typing import Protocol


class Driver(Protocol):
    def open(self, argv: list[str], env: dict[str, str], cwd: Path, font_size: float) -> str:
        """Open a new window running argv; returns the terminal id."""

    def send_keys(self, terminal: str, keys: list[str], interval: float = 0.0) -> None:
        """Keys: "j", "ctrl-d", "PageDown", "g", "r", "q". Sleeps interval seconds between keys."""

    def terminal_ids(self) -> set[str]:
        """Ids of every open terminal."""

    def terminal_tty(self, terminal: str) -> str | None:
        """The terminal's tty path ("/dev/ttys016"), None when it cannot be read."""

    def terminal_pid(self, terminal: str) -> int | None:
        """None once the terminal's process is gone."""

    def close(self, terminal: str) -> None: ...


class AutomationDenied(RuntimeError):
    """macOS refused Apple Events to Ghostty (error -1743)."""


class AppleScriptError(RuntimeError):
    pass


class DriverTimeout(RuntimeError):
    """An osascript call did not finish in time."""


_OPEN = """
on run argv
    set fontSize to (item 3 of argv) as real
    set envLines to paragraphs of (item 4 of argv)
    tell application "Ghostty"
        set cfg to new surface configuration from {font size:fontSize, command:(item 1 of argv), initial working directory:(item 2 of argv), environment variables:envLines, wait after command:false}
        set win to new window with configuration cfg
        return id of (focused terminal of selected tab of win)
    end tell
end run
"""

_SEND_KEYS = """
on run argv
    set termId to item 1 of argv
    set gap to (item 2 of argv) as real
    set total to count of argv
    tell application "Ghostty"
        repeat with i from 3 to total by 2
            set keyName to item i of argv
            set mods to item (i + 1) of argv
            if mods is "" then
                send key keyName to terminal id termId
            else
                send key keyName modifiers mods to terminal id termId
            end if
            if i + 1 < total and gap > 0 then delay gap
        end repeat
    end tell
end run
"""

_IDS = """
tell application "Ghostty"
    set AppleScript's text item delimiters to linefeed
    return (id of every terminal) as text
end tell
"""

_PID = """
on run argv
    tell application "Ghostty" to return pid of terminal id (item 1 of argv)
end run
"""

_TTY = """
on run argv
    tell application "Ghostty" to return tty of terminal id (item 1 of argv)
end run
"""

_CLOSE = """
on run argv
    tell application "Ghostty" to close terminal id (item 1 of argv)
end run
"""

_PROBE = 'tell application "Ghostty" to return version'

_KEY_NAMES = {"PageDown": "page_down"}


def _key_spec(key: str) -> tuple[str, str]:
    if key.startswith("ctrl-"):
        return key.removeprefix("ctrl-"), "control"
    return _KEY_NAMES.get(key, key), ""


def _osascript(script: str, *args: str, timeout: float = 15) -> str:
    try:
        result = subprocess.run(
            ["osascript", "-e", script, *args], capture_output=True, text=True, timeout=timeout
        )
    except subprocess.TimeoutExpired as error:
        raise DriverTimeout(f"osascript timed out after {timeout}s") from error
    if result.returncode != 0:
        if "-1743" in result.stderr:
            raise AutomationDenied(result.stderr.strip())
        raise AppleScriptError(result.stderr.strip())
    return result.stdout.strip()


class GhosttyDriver:
    def check_access(self) -> str:
        return _osascript(_PROBE)

    def open(self, argv: list[str], env: dict[str, str], cwd: Path, font_size: float) -> str:
        env_lines = "\n".join(f"{key}={value}" for key, value in env.items())
        return _osascript(_OPEN, shlex.join(argv), str(cwd), str(font_size), env_lines)

    def send_keys(self, terminal: str, keys: list[str], interval: float = 0.0) -> None:
        # One osascript call per sequence: each call costs 100+ ms, so pacing happens inside AppleScript.
        specs = [part for key in keys for part in _key_spec(key)]
        _osascript(_SEND_KEYS, terminal, str(interval), *specs, timeout=15 + len(keys) * interval)

    def terminal_ids(self) -> set[str]:
        return {line for line in _osascript(_IDS).splitlines() if line}

    def terminal_pid(self, terminal: str) -> int | None:
        try:
            return int(_osascript(_PID, terminal))
        except (AppleScriptError, ValueError):
            return None

    def terminal_tty(self, terminal: str) -> str | None:
        try:
            return _osascript(_TTY, terminal) or None
        except AppleScriptError:
            return None

    def close(self, terminal: str) -> None:
        try:
            _osascript(_CLOSE, terminal)
        except (AppleScriptError, DriverTimeout):
            pass  # the window already closed itself when the command exited
