#!/usr/bin/env python3
"""Build data/infoods/infoods_components.json from vendored FAO INFOODS sources.

Inputs (under data/infoods/raw/):
  PART1.TXT … PART5.TXT   — core tagnames (FAO plain text)
  TAGREV_2008.csv         — Oct 2008 additions (CSV only; never commit .xls)
  TAGREV_2010.csv         — Apr 2010 additions (CSV only)

No network access. Re-run after updating raw files.
"""

from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RAW = ROOT / "data" / "infoods" / "raw"
OUT = ROOT / "data" / "infoods" / "infoods_components.json"

# Entry openers only at line start (inline refs like <UNIT/> or mid-note <NA> are ignored).
ENTRY_RE = re.compile(r"^<([A-Z][A-Z0-9+\-]*)>", re.MULTILINE)
FIELD_RE = re.compile(
    r"^\s*(Unit|Synonyms|Comments|Tables|Note)\s*:\s*(.*)$", re.IGNORECASE
)
# Unit field often continues with prose after "mcg." / "mg." — keep only the first token.
UNIT_TOKEN_RE = re.compile(r"^([A-Za-zµμ%/\-]+(?:\s*[A-Za-z%]+)?)")


def clean_ws(s: str) -> str:
    return re.sub(r"\s+", " ", s).strip()


def parse_synonyms(blob: str) -> list[str]:
    if not blob:
        return []
    # Drop parenthetical notes like "(Note that these…)"
    blob = re.sub(r"\([^)]*\)", " ", blob)
    parts = re.split(r"[;,]|\bor\b", blob, flags=re.IGNORECASE)
    out: list[str] = []
    seen: set[str] = set()
    for p in parts:
        p = clean_ws(p)
        p = p.strip(" .")
        if len(p) < 2:
            continue
        # Skip pure noise
        if p.lower() in {"see", "unknown", "variable", "note", "notes"}:
            continue
        key = p.casefold()
        if key in seen:
            continue
        seen.add(key)
        out.append(p)
    return out


def normalize_unit(raw: str) -> str | None:
    raw = clean_ws(raw)
    if not raw:
        return None
    # Stop at sentence continuation after unit token.
    m = UNIT_TOKEN_RE.match(raw)
    if not m:
        return raw.split()[0] if raw.split() else None
    u = m.group(1).strip()
    # Common normalizations for matching later
    u_cf = u.casefold().replace("μ", "µ").replace("μ", "µ")
    if u_cf in {"mcg", "ug", "µg", "μg"}:
        return "µg"
    if u_cf in {"mg", "g", "kg", "kcal", "kj", "%", "iu"}:
        return u_cf if u_cf != "iu" else "IU"
    return u


def strip_inline_tags(s: str) -> str:
    """Remove angle-bracket tag refs so they don't pollute names."""
    return clean_ws(re.sub(r"<[^>]+>", " ", s))


def parse_part_txt(path: Path, source: str) -> list[dict]:
    text = path.read_text(encoding="latin-1", errors="replace")
    matches = list(ENTRY_RE.finditer(text))
    entries: list[dict] = []
    for i, m in enumerate(matches):
        tag = m.group(1)
        start = m.end()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        body = text[start:end]
        lines = body.splitlines()

        unit = ""
        synonyms_raw = ""
        comments = ""
        tables_note = ""
        name_parts: list[str] = []
        mode: str | None = None  # continuing multi-line field

        for line in lines:
            fm = FIELD_RE.match(line)
            if fm:
                mode = fm.group(1).lower()
                rest = fm.group(2).strip()
                if mode == "unit":
                    unit = rest
                    mode = None
                elif mode == "synonyms":
                    synonyms_raw = rest
                elif mode == "comments":
                    comments = rest
                elif mode == "note":
                    # Fold notes into comments
                    comments = (comments + " " + rest).strip() if comments else rest
                    mode = "comments"
                elif mode == "tables":
                    tables_note = rest
                continue
            stripped = line.strip()
            if not stripped:
                continue
            if mode == "synonyms":
                synonyms_raw = (synonyms_raw + " " + stripped).strip()
            elif mode == "comments":
                comments = (comments + " " + stripped).strip()
            elif mode == "tables":
                tables_note = (tables_note + " " + stripped).strip()
            else:
                # Name / description lines before first field
                name_parts.append(stripped)

        name = strip_inline_tags(" ".join(name_parts))
        if not name or name.casefold() in {"must be", "must be explicitly stated with the secondary tagname"}:
            name = tag
        # Truncate pathological long names at first semicolon if still huge? keep full cleaned name
        if len(name) > 200:
            name = name[:200].rsplit(" ", 1)[0]
        synonyms = parse_synonyms(synonyms_raw)
        entries.append(
            {
                "tag": tag,
                "name": name,
                "unit": normalize_unit(unit),
                "synonyms": synonyms,
                "comments": strip_inline_tags(comments) or None,
                "tables_note": clean_ws(tables_note) or None,
                "source": source,
            }
        )
    return entries


def parse_addition_csv(path: Path, source: str) -> list[dict]:
    """CSV columns: TAGNAME, Short description, Description, Recommended units, Comment, SYNONYMS"""
    entries: list[dict] = []
    with path.open(encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            # normalize header keys
            row = { (k or "").strip(): (v or "").strip() for k, v in row.items() }
            tag = row.get("TAGNAME") or row.get("Tagname") or row.get("tag")
            if not tag:
                continue
            tag = tag.strip()
            if not re.fullmatch(r"[A-Za-z][A-Za-z0-9+\-]*", tag):
                continue
            tag = tag.upper() if tag.isupper() or tag.islower() else tag
            # Prefer Description, fall back to Short description
            name = row.get("Description") or row.get("Short description") or tag
            name = clean_ws(name)
            unit = clean_ws(row.get("Recommended units") or "") or None
            comments = clean_ws(row.get("Comment") or "") or None
            synonyms = parse_synonyms(row.get("SYNONYMS") or row.get("Synonyms") or "")
            # Also add short description as synonym if distinct
            short = clean_ws(row.get("Short description") or "")
            if short and short.casefold() != name.casefold():
                if short.casefold() not in {s.casefold() for s in synonyms}:
                    synonyms.insert(0, short)
            entries.append(
                {
                    "tag": tag,
                    "name": name,
                    "unit": unit,
                    "synonyms": synonyms,
                    "comments": comments,
                    "tables_note": None,
                    "source": source,
                }
            )
    return entries


def merge_entries(all_entries: list[dict]) -> list[dict]:
    """Later sources win on field fill; first tag occurrence keeps order.

    Core PART files first, then 2008, then 2010 additions.
    """
    by_tag: dict[str, dict] = {}
    order: list[str] = []
    for e in all_entries:
        key = e["tag"].upper()
        if key not in by_tag:
            by_tag[key] = {
                "tag": e["tag"].upper() if e["tag"].isascii() else e["tag"],
                "name": e["name"],
                "unit": e["unit"],
                "synonyms": list(e["synonyms"]),
                "comments": e["comments"],
                "tables_note": e["tables_note"],
                "source": e["source"],
            }
            order.append(key)
        else:
            cur = by_tag[key]
            # Prefer richer name if current is just the tag
            if cur["name"].casefold() == key.casefold() and e["name"]:
                cur["name"] = e["name"]
            if not cur["unit"] and e["unit"]:
                cur["unit"] = e["unit"]
            if not cur["comments"] and e["comments"]:
                cur["comments"] = e["comments"]
            if not cur["tables_note"] and e["tables_note"]:
                cur["tables_note"] = e["tables_note"]
            # merge synonyms
            seen = {s.casefold() for s in cur["synonyms"]}
            for s in e["synonyms"]:
                if s.casefold() not in seen:
                    cur["synonyms"].append(s)
                    seen.add(s.casefold())
            # track last source that touched it
            cur["source"] = e["source"] if e["source"].startswith("add") else cur["source"]
    # Normalize tag to uppercase for ASCII tags
    out = []
    for key in order:
        e = by_tag[key]
        e["tag"] = key  # uppercase
        out.append(e)
    return out


def main() -> int:
    if not RAW.is_dir():
        print(f"missing raw dir: {RAW}", file=sys.stderr)
        return 1

    entries: list[dict] = []
    for i in range(1, 6):
        path = RAW / f"PART{i}.TXT"
        if not path.is_file():
            print(f"missing {path}", file=sys.stderr)
            return 1
        part = parse_part_txt(path, f"part{i}")
        print(f"{path.name}: {len(part)} entries")
        entries.extend(part)

    for name, source in [
        ("TAGREV_2008.csv", "add2008"),
        ("TAGREV_2010.csv", "add2010"),
    ]:
        path = RAW / name
        if not path.is_file():
            print(f"missing {path}", file=sys.stderr)
            return 1
        add = parse_addition_csv(path, source)
        print(f"{path.name}: {len(add)} entries")
        entries.extend(add)

    merged = merge_entries(entries)
    print(f"merged unique tags: {len(merged)}")

    # Sanity: classics present
    tags = {e["tag"] for e in merged}
    for need in ["VITC", "CA", "FE", "ZN", "MG", "NIA", "THIA", "RIBF"]:
        if need not in tags:
            print(f"WARNING: expected tag {need} missing", file=sys.stderr)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        json.dumps(
            {
                "version": 1,
                "description": "FAO INFOODS food component tagnames (vendored)",
                "components": merged,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
