#!/usr/bin/env python3
"""Generate the selected non-text layout subset from Open XML SDK schema JSON.

Source: dotnet/Open-XML-SDK 431ab05cf160248cc3885a4a766026d4f8243792 (MIT).
Pass its data/schemas directory; pipe stdout through rustfmt --emit stdout. Selection is explicit;
new schema elements are never admitted by namespace or attribute type alone.
"""
import json
from pathlib import Path
import sys

SCHEMAS = Path(sys.argv[1])
FAMILIES = {
    "a": "left right top bottom highlight blipFill spPr cNvPicPr gradFill lin tile pattFill grpFill alphaBiLevel alphaCeiling alphaFloor alphaInv alphaMod alphaModFix alphaOutset alphaRepl biLevel blend blur clrChange clrRepl duotone fillOverlay glow grayscl hsl innerShdw lum outerShdw prstShdw reflection relOff softEdge hue hueOff alphaOff gamma invGamma comp inv gray scrgbClr hslClr schemeClr prstClr prstGeom prstTxWarp round bevel miter prstDash custDash buClrTx buSzTx buSzPct buSzPts buFontTx buNone buAutoNum uLnTx uFillTx uFill noAutofit normAutofit spAutoFit spLocks cxnSpLocks graphicFrameLocks grpSpLocks picLocks tab spcPct spcPts tabLst gridCol tblGrid tr tc tbl tcPr tcBdr cell3D fill lnL lnR lnT lnB lnTlToBr lnBlToTr",
    "x": "workbookPr vertical horizontal",
    "w": "noWrap noEndnote hideMark tblLayout tblpPr tblOverlap tblInd tblCellMar tcMar trHeight hMerge vMerge textAlignment textboxTightWrap pgBorders ptab bdo dir",
    "p": "notes penClr spPr cNvPicPr blipFill",
    "pic": "spPr cNvPicPr blipFill",
    "xdr": "spPr cNvPicPr blipFill",
}
NS = {"a": "A", "w": "W", "p": "P", "pic": "PIC", "xdr": "XDR", "x": "S"}
all_data = [json.loads(p.read_text()) for p in sorted(SCHEMAS.glob("*.json"))]
enums = {e["Name"]: [f["Value"] for f in e["Facets"]] for d in all_data for e in d.get("Enums", [])}
integers = {"Int32Value":"i32", "UInt32Value":"u32", "Int64Value":"i64", "UInt64Value":"u64", "Int16Value":"i16", "UInt16Value":"u16", "ByteValue":"u8", "SByteValue":"i8"}
rules = {}
ambiguous = set()
for d in all_data:
    for t in d.get("Types", []):
        name = t["Name"].split("/")[-1]
        if ":" not in name:
            continue
        prefix, element = name.split(":")
        if prefix not in FAMILIES or element not in FAMILIES[prefix].split():
            continue
        # Only non-text schema bases are eligible, including inherited typed flags.
        ancestry = []
        base = t
        while base.get("BaseClass") not in ("OpenXmlCompositeElement", "OpenXmlLeafElement"):
            ancestry.extend(base.get("Attributes", []))
            base = next((b for b in d.get("Types", []) if b["ClassName"] == base.get("BaseClass")), None)
            if base is None:
                break
        if base is None:
            continue
        ancestry.extend(base.get("Attributes", []))
        attrs = []
        for a in ancestry:
            if (prefix, element) == ("x", "workbookPr") and a["QName"].split(":")[-1] not in "date1904 dateCompatibility showObjects showBorderUnselectedTables filterPrivacy showInkAnnotation hidePivotFieldList showPivotChartFilter autoCompressPictures defaultThemeVersion".split():
                continue
            key, ty = a["QName"].split(":")[-1], a["Type"]
            if key.lower().endswith("id") or key in ("name", "uri", "target"):
                break
            if ty in integers:
                valid = f"value.parse::<{integers[ty]}>().is_ok()"
            elif ty in ("BooleanValue", "OnOffOnlyValue"):
                valid = 'matches!(value, "true" | "false" | "0" | "1")'
            elif ty == "OnOffValue":
                valid = 'matches!(value, "true" | "false" | "0" | "1" | "on" | "off")'
            elif ty == "DoubleValue":
                valid = 'super::plain_number(value)'
            elif ty.startswith("EnumValue<"):
                values = enums[ty[:-1].split(".")[-1]]
                valid = "matches!(value, " + " | ".join(json.dumps(v) for v in values) + ")"
            else:
                break
            attrs.append((key, valid))
        else:
            k = (NS[prefix], element)
            if k in rules and rules[k] != attrs:
                ambiguous.add(k)
            rules[k] = attrs

for key in ambiguous:
    del rules[key]

print("//! Generated selected layout subset; see scripts/office-layout-allowlist.py.")
print("//! Open XML SDK 431ab05cf160248cc3885a4a766026d4f8243792 (MIT; OPEN_XML_SDK_LICENSE.txt).")
print("//! No string, identifier, relationship, target or text-bearing attributes.")
print("use super::{A, W, P, PIC, XDR, S};")
print("pub(super) fn rule(ns: &str, element: &str) -> Option<&'static str> {\n    Some(match (ns, element) {")
for (ns, el), attrs in sorted(rules.items()):
    print(f'        ({ns}, "{el}") => "' + " ".join(k for k, _ in attrs) + '",')
print("        _ => return None,\n    })\n}")
print("pub(super) fn valid(ns: &str, element: &str, attr: &str, value: &str) -> bool {\n    match (ns, element, attr) {")
for (ns, el), attrs in sorted(rules.items()):
    for key, valid in attrs:
        print(f'        ({ns}, "{el}", "{key}") => {valid},')
print("        _ => false,\n    }\n}")
