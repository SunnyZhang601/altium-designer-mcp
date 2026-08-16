"""Pre-flight check for GenerateSamples.pas before spending an Altium run.

DelphiScript resolves every property name at COMPILE time, per interface. One
name that the interface does not carry aborts the whole script — every other
footprint and symbol in that run is lost, and `try/except` cannot help, because
nothing has run yet. Two of these cost a full round trip each: `Arc.IsSolid`
(an ISch_Arc is a stroked shape with no fill) and `Pie.Transparent`.

The script engine's identifier table is not enough on its own to prevent that.
It lives in `ScriptingSystem.dll` as UTF-16LE strings and is worth checking --
a name absent from it does not exist at all -- but it is GLOBAL: a hit only
says the name is real somewhere, not that it is real on the interface being
assigned. `IsSolid` is in the table and is genuine on a rectangle.

So this checks the other half, from the file itself: every `(interface,
property)` pair assigned in live code that has no precedent in the committed
version is unproven, and is a compile-abort risk. Discipline that follows from
that: introduce at most ONE unproven family per run, so a failure costs one
round trip and names one identifier.

The committed file is the baseline because a pair only reaches it after a run
that actually compiled -- commit the regenerated binaries and the `.pas`
together, and never commit an assignment Altium has not accepted.

    python scripts/altium/generate/preflight_names.py

Exits 0 when nothing is unproven, 1 otherwise -- a non-zero exit is a prompt to
check the list, not necessarily a defect. Comments (`{ }`, `(* *)`, `//`) are
stripped first, so a staged probe parked in a comment block is correctly
ignored.
"""

import re
import subprocess
import sys

PAS = "scripts/altium/generate/GenerateSamples.pas"


def strip_comments(src: str) -> str:
    """Remove `(* *)`, `{ }` and `//` comments so staged probes are ignored."""
    src = re.sub(r"\(\*.*?\*\)", "", src, flags=re.S)
    src = re.sub(r"\{[^{}]*\}", "", src)
    return re.sub(r"//[^\n]*", "", src)


def assigned_pairs(src: str) -> set:
    """-> {(interface_type, property)} for every live assignment in `src`."""
    pairs = set()
    # Split on routine boundaries so one routine's `var` block cannot be read
    # as declaring another's locals.
    for routine in re.split(r"\n(?=(?:procedure|function)\s)", strip_comments(src)):
        declared = dict(re.findall(r"(\w+)\s*:\s*(ISch_\w+|IPCB_\w+)", routine))
        for var, prop in re.findall(r"\b(\w+)\.(\w+)\s*:=", routine):
            if var in declared:
                pairs.add((declared[var], prop))
    return pairs


def main() -> int:
    head = subprocess.run(
        ["git", "show", f"HEAD:{PAS}"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=True,
    ).stdout
    with open(PAS, encoding="utf-8") as handle:
        working = handle.read()

    proven = assigned_pairs(head)
    unproven = sorted(assigned_pairs(working) - proven)

    if not unproven:
        print(f"OK: every live assignment has a precedent ({len(proven)} known pairs).")
        return 0

    families = {iface for iface, _ in unproven}
    print(f"{len(unproven)} unproven (interface, property) pair(s) across "
          f"{len(families)} interface(s) -- each risks aborting the run:\n")
    for iface, prop in unproven:
        elsewhere = sorted({i for i, p in proven if p == prop})
        note = (
            f"proven on {', '.join(elsewhere)} -- but NOT on this interface"
            if elsewhere
            else "name never assigned in this file"
        )
        print(f"    {iface:24}.{prop:<20} {note}")

    if len(families) > 1:
        print(
            f"\nMore than one interface is unproven. A failure would not say which "
            f"is at fault; stage all but one behind a `(* *)` comment first."
        )
    return 1


if __name__ == "__main__":
    sys.exit(main())
