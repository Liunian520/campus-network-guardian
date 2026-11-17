#!/usr/bin/env python3
"""Audit the actual native UI color tokens, without launching the application.

The input is src/ui_theme.rs, not a second hand-maintained palette. Rust rgb()
arguments are ordinary RGB bytes; direct u32 literals are Windows COLORREF
(0x00BBGGRR). Unsupported expressions fail explicitly rather than being guessed.
This checks palette pairs only; layout, high contrast, focus geometry, and native
accessibility semantics still require their own inspection.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
import unittest
from dataclasses import dataclass


ROOT = Path(__file__).resolve().parent.parent
TOKEN = re.compile(
    r"(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*u32\s*=\s*([^;]+);"
)
NUMBER = re.compile(r"(?:0x[0-9a-fA-F_]+|[0-9][0-9_]*)(?:u8|u32)?\Z")


def integer(value: str) -> int:
    value = value.strip()
    if not NUMBER.fullmatch(value):
        raise ValueError(f"unsupported Rust integer: {value!r}")
    value = re.sub(r"u(?:8|32)$", "", value).replace("_", "")
    return int(value, 16 if value.startswith("0x") else 10)


def parse_tokens(source: str) -> dict[str, tuple[int, int, int]]:
    source = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    source = re.sub(r"//[^\n]*", "", source)
    expressions: dict[str, str] = {}
    for name, expression in TOKEN.findall(source):
        if name in expressions:
            raise ValueError(f"duplicate color token: {name}")
        expressions[name] = expression.strip()
    result: dict[str, tuple[int, int, int]] = {}

    def resolve(name: str, chain: tuple[str, ...] = ()) -> tuple[int, int, int]:
        if name in result:
            return result[name]
        if name in chain:
            raise ValueError(f"cyclic color token: {' -> '.join((*chain, name))}")
        if name not in expressions:
            raise ValueError(f"missing color token: {name}")
        expression = expressions[name]
        call = re.fullmatch(r"rgb\(\s*(.*?)\s*\)", expression, flags=re.S)
        if call:
            parts = [integer(part) for part in call.group(1).split(",") if part.strip()]
            if len(parts) != 3 or any(not 0 <= part <= 255 for part in parts):
                raise ValueError(f"{name}: rgb() must contain three byte values")
            color = tuple(parts)
        elif re.fullmatch(r"[A-Z][A-Z0-9_]*", expression):
            color = resolve(expression, (*chain, name))
        else:
            value = integer(expression)
            if not 0 <= value <= 0xFFFFFF:
                raise ValueError(f"{name}: COLORREF is outside 24-bit color range")
            color = (value & 255, (value >> 8) & 255, (value >> 16) & 255)
        result[name] = color
        return color

    for name in expressions:
        resolve(name)
    if not result:
        raise ValueError("no u32 color tokens found")
    return result


def luminance(color: tuple[int, int, int]) -> float:
    def linear(byte: int) -> float:
        value = byte / 255
        return value / 12.92 if value <= 0.04045 else ((value + 0.055) / 1.055) ** 2.4

    return sum(linear(byte) * weight for byte, weight in zip(color, (0.2126, 0.7152, 0.0722)))


def contrast(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    light, dark = sorted((luminance(a), luminance(b)), reverse=True)
    return (light + 0.05) / (dark + 0.05)


def hex_color(color: tuple[int, int, int]) -> str:
    return "#" + "".join(f"{byte:02X}" for byte in color)


@dataclass(frozen=True)
class Pair:
    label: str
    foreground: str
    background: str
    minimum: float | None


# These are semantic uses, not duplicated color values. Keep the names aligned
# with the controls which use the corresponding tokens in windows_ui.rs.
PAIRS = [
    Pair("body text / card", "TEXT_PRIMARY", "SURFACE", 4.5),
    Pair("body text / page", "TEXT_PRIMARY", "BACKGROUND", 4.5),
    Pair("body text / sidebar", "TEXT_PRIMARY", "SIDEBAR", 4.5),
    Pair("secondary / card", "TEXT_SECONDARY", "SURFACE", 4.5),
    Pair("secondary / page", "TEXT_SECONDARY", "BACKGROUND", 4.5),
    Pair("secondary / sidebar", "TEXT_SECONDARY", "SIDEBAR", 4.5),
    Pair("tertiary / card", "TEXT_TERTIARY", "SURFACE", 4.5),
    Pair("tertiary / page", "TEXT_TERTIARY", "BACKGROUND", 4.5),
    Pair("tertiary / sidebar", "TEXT_TERTIARY", "SIDEBAR", 4.5),
    Pair("hero headline", "TEXT_ON_HERO", "HERO", 4.5),
    Pair("hero secondary", "TEXT_HERO_SECONDARY", "HERO", 4.5),
    Pair("primary normal", "TEXT_ON_PRIMARY", "PRIMARY", 4.5),
    Pair("primary hover", "TEXT_ON_PRIMARY", "PRIMARY_HOVER", 4.5),
    Pair("primary pressed", "TEXT_ON_PRIMARY", "PRIMARY_PRESSED", 4.5),
    Pair("selected navigation", "NAV_SELECTED_TEXT", "PRIMARY_SUBTLE", 4.5),
    Pair("selected navigation pressed", "NAV_SELECTED_PRESSED_TEXT", "NAV_PRESSED", 4.5),
    Pair("navigation hover", "TEXT_SECONDARY", "NAV_HOVER", 4.5),
    Pair("navigation pressed", "TEXT_SECONDARY", "NAV_PRESSED", 4.5),
    Pair("author link normal", "PRIMARY", "SIDEBAR", 4.5),
    Pair("author link hover", "PRIMARY_HOVER", "NAV_HOVER", 4.5),
    Pair("author link pressed", "PRIMARY_PRESSED", "NAV_PRESSED", 4.5),
    Pair("sidebar footer hover", "TEXT_TERTIARY", "NAV_HOVER", 4.5),
    Pair("sidebar footer pressed", "TEXT_SECONDARY", "NAV_PRESSED", 4.5),
    Pair("neutral button hover", "TEXT_SECONDARY", "NEUTRAL_HOVER", 4.5),
    Pair("neutral button pressed", "TEXT_SECONDARY", "NEUTRAL_PRESSED", 4.5),
    Pair("delete button normal", "TEXT_TERTIARY", "BACKGROUND", 4.5),
    Pair("delete button hover", "DANGER_TEXT", "NEUTRAL_HOVER", 4.5),
    Pair("delete button pressed", "DANGER_TEXT", "NEUTRAL_PRESSED", 4.5),
    Pair("delete keyboard pressed", "TEXT_TERTIARY", "NEUTRAL_PRESSED", 4.5),
    Pair("input/button boundary", "CONTROL_BORDER", "SURFACE", 3.0),
    Pair("button boundary / page", "CONTROL_BORDER", "BACKGROUND", 3.0),
    Pair("input focus / card", "FOCUS", "SURFACE", 3.0),
    Pair("button focus / page", "FOCUS", "BACKGROUND", 3.0),
    Pair("neutral focus / hover", "FOCUS", "NEUTRAL_HOVER", 3.0),
    Pair("neutral focus / pressed", "FOCUS", "NEUTRAL_PRESSED", 3.0),
    Pair("nav focus / selected", "FOCUS", "PRIMARY_SUBTLE", 3.0),
    Pair("nav focus / hover", "FOCUS", "NAV_HOVER", 3.0),
    Pair("nav focus / pressed", "FOCUS", "NAV_PRESSED", 3.0),
    Pair("primary focus / normal", "FOCUS_ON_PRIMARY", "PRIMARY", 3.0),
    Pair("primary focus / hover", "FOCUS_ON_PRIMARY", "PRIMARY_HOVER", 3.0),
    Pair("primary focus / pressed", "FOCUS_ON_PRIMARY", "PRIMARY_PRESSED", 3.0),
    Pair("inline form error", "DANGER_TEXT", "BACKGROUND", 4.5),
    Pair("invalid input boundary", "DANGER_TEXT", "SURFACE", 3.0),
    Pair("decorative card outline", "SURFACE_BORDER", "SURFACE", None),
    Pair("disabled label (exempt)", "DISABLED_TEXT", "DISABLED_SURFACE", None),
]


def audit(tokens: dict[str, tuple[int, int, int]]) -> list[dict]:
    rows = []
    for pair in PAIRS:
        for name in (pair.foreground, pair.background):
            if name not in tokens:
                raise ValueError(f"{pair.label}: missing color token {name}")
        foreground, background = tokens[pair.foreground], tokens[pair.background]
        ratio = contrast(foreground, background)
        rows.append({
            "label": pair.label,
            "foreground_token": pair.foreground,
            "foreground": hex_color(foreground),
            "background_token": pair.background,
            "background": hex_color(background),
            "ratio": ratio,
            "minimum": pair.minimum,
            "pass": pair.minimum is None or ratio >= pair.minimum,
        })
    return rows


class AuditSelfTest(unittest.TestCase):
    def test_reference_luminance_ratios(self) -> None:
        self.assertAlmostEqual(contrast((0, 0, 0), (255, 255, 255)), 21)
        self.assertAlmostEqual(contrast((153, 153, 153), (255, 255, 255)), 2.849027755, places=7)
        self.assertEqual(contrast((47, 83, 129), (47, 83, 129)), 1)

    def test_rust_rgb_and_colorref_resolve_to_same_rgb(self) -> None:
        tokens = parse_tokens("""
            pub const RGB: u32 = rgb(0x12, 0x34, 0x56);
            const COLORREF: u32 = 0x00_56_34_12;
            pub(crate) const ALIAS: u32 = RGB;
        """)
        self.assertEqual(tokens["RGB"], (0x12, 0x34, 0x56))
        self.assertEqual(tokens["RGB"], tokens["COLORREF"])
        self.assertEqual(tokens["RGB"], tokens["ALIAS"])

    def test_malformed_and_unresolved_colors_fail_closed(self) -> None:
        for source in (
            "const BAD: u32 = rgb(256, 0, 0);",
            "const BAD: u32 = UNKNOWN;",
            "const BAD: u32 = 0x12 + 3;",
            "const A: u32 = B; const B: u32 = A;",
            "const A: u32 = 0; const A: u32 = 1;",
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                parse_tokens(source)


def main() -> int:
    # PowerShell callers receive UTF-8, including repository paths in Chinese.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    if hasattr(sys.stderr, "reconfigure"):
        sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--theme", type=Path, default=ROOT / "src" / "ui_theme.rs")
    parser.add_argument("--json", action="store_true", help="emit machine-readable measured pairs")
    parser.add_argument("--self-test", action="store_true", help="check parser and contrast math without reading UI files")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(AuditSelfTest)
        return 0 if unittest.TextTestRunner().run(suite).wasSuccessful() else 1
    try:
        tokens = parse_tokens(args.theme.read_text(encoding="utf-8"))
        rows = audit(tokens)
    except (OSError, ValueError) as error:
        print(f"contrast audit error: {error}", file=sys.stderr)
        return 2
    passed = all(row["pass"] for row in rows)
    if args.json:
        print(json.dumps({"theme": str(args.theme.resolve()), "pass": passed, "pairs": rows}, indent=2))
    else:
        print(f"Native UI contrast audit: {args.theme.resolve()}")
        for row in rows:
            threshold = f">={row['minimum']:.1f}" if row["minimum"] is not None else "informational"
            verdict = "PASS" if row["pass"] else "FAIL"
            print(f"{verdict:4} {row['label']:29} {row['foreground']} / {row['background']}  {row['ratio']:5.2f}:1  {threshold}")
        print(f"{sum(row['pass'] for row in rows)}/{len(rows)} pairs accepted; decorative/disabled rows are informational.")
        print("This palette audit does not certify screen-reader support, focus geometry, or DPI layout.")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
