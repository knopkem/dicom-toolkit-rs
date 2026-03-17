#!/usr/bin/env python3
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

VARIABLE_VM = "u32::MAX"


@dataclass(frozen=True, order=True)
class EntryKey:
    lower_group: int
    lower_element: int
    upper_group: int
    upper_element: int
    group_restriction: str
    element_restriction: str
    private_creator: str | None


@dataclass(frozen=True)
class ParsedEntry:
    lower_group: int
    lower_element: int
    upper_group: int
    upper_element: int
    group_restriction: str
    element_restriction: str
    raw_vr: str
    resolved_vr: str
    keyword: str
    vm_min: int
    vm_max: int | None
    standard_version: str
    private_creator: str | None

    @property
    def key(self) -> EntryKey:
        return EntryKey(
            self.lower_group,
            self.lower_element,
            self.upper_group,
            self.upper_element,
            self.group_restriction,
            self.element_restriction,
            self.private_creator,
        )

    @property
    def is_repeating(self) -> bool:
        return (
            self.lower_group != self.upper_group
            or self.lower_element != self.upper_element
        )


STANDARD_VR_MAP = {
    "AE": "Vr::AE",
    "AS": "Vr::AS",
    "AT": "Vr::AT",
    "CS": "Vr::CS",
    "DA": "Vr::DA",
    "DS": "Vr::DS",
    "DT": "Vr::DT",
    "FD": "Vr::FD",
    "FL": "Vr::FL",
    "IS": "Vr::IS",
    "LO": "Vr::LO",
    "LT": "Vr::LT",
    "OB": "Vr::OB",
    "OD": "Vr::OD",
    "OF": "Vr::OF",
    "OL": "Vr::OL",
    "OV": "Vr::OV",
    "OW": "Vr::OW",
    "PN": "Vr::PN",
    "SH": "Vr::SH",
    "SL": "Vr::SL",
    "SQ": "Vr::SQ",
    "SS": "Vr::SS",
    "ST": "Vr::ST",
    "SV": "Vr::SV",
    "TM": "Vr::TM",
    "UC": "Vr::UC",
    "UI": "Vr::UI",
    "UL": "Vr::UL",
    "UN": "Vr::UN",
    "UR": "Vr::UR",
    "US": "Vr::US",
    "UT": "Vr::UT",
    "UV": "Vr::UV",
}

PSEUDO_VR_MAP = {
    "up": "Vr::UL",
    "xs": "Vr::US",
    "lt": "Vr::OW",
    "ox": "Vr::OB",
    "px": "Vr::OB",
    "na": "Vr::UN",
}

RANGE_RESTRICTION_MAP = {
    "unspecified": "RangeRestriction::Unspecified",
    "odd": "RangeRestriction::Odd",
    "even": "RangeRestriction::Even",
}


def rust_string(value: str) -> str:
    escaped = value.encode("unicode_escape").decode("ascii")
    escaped = escaped.replace('\\"', '\\\\"')
    return f'"{escaped}"'


def parse_hex(value: str) -> int:
    return int(value, 16)


def parse_tag_part(part: str) -> tuple[int, int, str]:
    text = "".join(part.split())
    pieces = text.split("-")
    if len(pieces) == 1:
        value = parse_hex(pieces[0])
        return value, value, "unspecified"
    if len(pieces) == 2:
        return parse_hex(pieces[0]), parse_hex(pieces[1]), "even"
    if len(pieces) == 3:
        low, restriction, high = pieces
        restriction = restriction.lower()
        if restriction == "o":
            restriction_name = "odd"
        elif restriction == "e":
            restriction_name = "even"
        elif restriction == "u":
            restriction_name = "unspecified"
        else:
            raise ValueError(f"unknown range restriction: {part!r}")
        return parse_hex(low), parse_hex(high), restriction_name
    raise ValueError(f"invalid tag range part: {part!r}")


def parse_tag_field(field: str) -> tuple[int, int, int, int, str, str, str | None]:
    text = field.strip()
    if not (text.startswith("(") and text.endswith(")")):
        raise ValueError(f"invalid tag field: {field!r}")
    inner = text[1:-1]
    if "," not in inner:
        raise ValueError(f"invalid tag field: {field!r}")
    group_part, rest = inner.split(",", 1)
    rest = rest.strip()
    private_creator = None
    if rest.startswith('"'):
        end_quote = rest.find('"', 1)
        if end_quote == -1:
            raise ValueError(f"unterminated private creator in tag field: {field!r}")
        private_creator = rest[1:end_quote]
        rest = rest[end_quote + 1 :].strip()
        if not rest.startswith(","):
            raise ValueError(f"missing element part in tag field: {field!r}")
        rest = rest[1:].strip()
    element_part = rest
    lg, ug, group_restriction = parse_tag_part(group_part)
    le, ue, element_restriction = parse_tag_part(element_part)
    return lg, le, ug, ue, group_restriction, element_restriction, private_creator


def parse_vm(field: str) -> tuple[int, int | None]:
    text = "".join(field.split()).lower()
    if text == "n":
        return 1, None
    if "-" in text:
        low, high = text.split("-", 1)
        vm_min = int(low)
        if high.endswith("n"):
            return vm_min, None
        return vm_min, int(high)
    if text.endswith("n"):
        prefix = text[:-1]
        return (int(prefix) if prefix else 1), None
    value = int(text)
    return value, value


def resolve_vr(raw_vr: str) -> str:
    if raw_vr in STANDARD_VR_MAP:
        return STANDARD_VR_MAP[raw_vr]
    if raw_vr in PSEUDO_VR_MAP:
        return PSEUDO_VR_MAP[raw_vr]
    raise ValueError(f"unsupported VR code in dictionary: {raw_vr!r}")


def parse_dictionary(paths: Iterable[Path]) -> list[ParsedEntry]:
    entries: dict[EntryKey, ParsedEntry] = {}
    for path in paths:
        with path.open("r", encoding="utf-8") as handle:
            for line_no, raw_line in enumerate(handle, start=1):
                stripped = raw_line.strip()
                if not stripped:
                    continue
                if stripped.startswith("#"):
                    continue
                fields = [field.strip() for field in raw_line.rstrip("\n").split("\t")]
                if len(fields) < 5:
                    raise ValueError(f"{path}:{line_no}: expected 5 tab-separated fields, got {len(fields)}")
                tag_field, raw_vr, keyword, vm_field, version = fields[:5]
                (
                    lower_group,
                    lower_element,
                    upper_group,
                    upper_element,
                    group_restriction,
                    element_restriction,
                    private_creator,
                ) = parse_tag_field(tag_field)
                vm_min, vm_max = parse_vm(vm_field)
                entry = ParsedEntry(
                    lower_group=lower_group,
                    lower_element=lower_element,
                    upper_group=upper_group,
                    upper_element=upper_element,
                    group_restriction=group_restriction,
                    element_restriction=element_restriction,
                    raw_vr=raw_vr,
                    resolved_vr=resolve_vr(raw_vr),
                    keyword=keyword,
                    vm_min=vm_min,
                    vm_max=vm_max,
                    standard_version=version,
                    private_creator=private_creator,
                )
                entries[entry.key] = entry
    return [entries[key] for key in sorted(entries)]


def emit_entry(entry: ParsedEntry) -> str:
    vm_max = VARIABLE_VM if entry.vm_max is None else str(entry.vm_max)
    private_creator = (
        f"Some({rust_string(entry.private_creator)})"
        if entry.private_creator is not None
        else "None"
    )
    return (
        "    DictEntry {\n"
        f"        tag: Tag::new(0x{entry.lower_group:04X}, 0x{entry.lower_element:04X}),\n"
        f"        upper_tag: Tag::new(0x{entry.upper_group:04X}, 0x{entry.upper_element:04X}),\n"
        f"        vr: {entry.resolved_vr},\n"
        f"        raw_vr: {rust_string(entry.raw_vr)},\n"
        f"        name: {rust_string(entry.keyword)},\n"
        f"        keyword: {rust_string(entry.keyword)},\n"
        f"        vm_min: {entry.vm_min},\n"
        f"        vm_max: {vm_max},\n"
        f"        standard_version: {rust_string(entry.standard_version)},\n"
        f"        group_restriction: {RANGE_RESTRICTION_MAP[entry.group_restriction]},\n"
        f"        element_restriction: {RANGE_RESTRICTION_MAP[entry.element_restriction]},\n"
        f"        private_creator: {private_creator},\n"
        "    },\n"
    )


def emit(entries: list[ParsedEntry], inputs: list[Path]) -> str:
    exact = [entry for entry in entries if not entry.is_repeating]
    repeating = [entry for entry in entries if entry.is_repeating]
    header_inputs = ", ".join(str(path) for path in inputs)
    lines = [
        "//! Generated from DCMTK dictionary sources.\n",
        f"//! Sources: {header_inputs}\n",
        f"//! Exact entries: {len(exact)}, repeating entries: {len(repeating)}\n",
        "\n",
        "use super::{DictEntry, RangeRestriction, Tag};\n",
        "use crate::vr::Vr;\n",
        "\n",
        "#[rustfmt::skip]\n",
        "pub(crate) static EXACT_ENTRIES: &[DictEntry] = &[\n",
    ]
    lines.extend(emit_entry(entry) for entry in exact)
    lines.append("];\n\n")
    lines.extend([
        "#[rustfmt::skip]\n",
        "pub(crate) static REPEATING_ENTRIES: &[DictEntry] = &[\n",
    ])
    lines.extend(emit_entry(entry) for entry in repeating)
    lines.append("];\n")
    return "".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate dicom-toolkit-dict tables from DCMTK .dic files")
    parser.add_argument("inputs", nargs="+", help="Input DCMTK .dic file(s)")
    parser.add_argument("-o", "--output", required=True, help="Output Rust file")
    args = parser.parse_args()

    input_paths = [Path(path).resolve() for path in args.inputs]
    output_path = Path(args.output).resolve()
    entries = parse_dictionary(input_paths)
    output_path.write_text(emit(entries, input_paths), encoding="utf-8")
    print(f"generated {output_path} from {len(input_paths)} source file(s) with {len(entries)} entries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
