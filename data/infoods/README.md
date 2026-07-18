# INFOODS food component tagnames (vendored)

Reference data from the FAO **International Network of Food Data Systems (INFOODS)**
[Food Component Identifiers (Tagnames)](https://www.fao.org/infoods/infoods/standards-guidelines/food-component-identifiers-tagnames/en/).

## Files

| Path | Role |
|------|------|
| `raw/PART1.TXT` … `raw/PART5.TXT` | Core tagnames (official plain text) |
| `raw/TAGREV_2008.csv` | Oct 2008 additions (**CSV only** — never commit `.xls`) |
| `raw/TAGREV_2010.csv` | Apr 2010 additions (**CSV only** — never commit `.xls`) |
| `infoods_components.json` | Canonical artifact loaded by recomplog migrations |

## Regenerating

```bash
# If FAO only publishes additions as .xls, convert offline first, e.g.:
#   ssconvert Tagname_new_April_2010-web__2_.xls data/infoods/raw/TAGREV_2010.csv
#   ssconvert TAGREV__1_.xls data/infoods/raw/TAGREV_2008.csv

python3 scripts/import_infoods.py
```

Do **not** fetch FAO at `cargo test` or runtime. Update raw files + re-run the script when revising the list.

## Attribution

Tagnames prepared from Klensin et al., *Identification of Food Components for INFOODS Data Interchange*, UNU, 1989; updated by INFOODS/FAO. See the FAO INFOODS site for terms of use.

Retrieved for this project: 2026-07-18.
