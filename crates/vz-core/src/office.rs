//! Namespace-aware Office package inspection. No network or formula evaluator.
use anyhow::{bail, ensure, Context, Result};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
};

mod chart_allowlist;
mod diagram_allowlist;
mod graphics;
mod layout_allowlist;
mod style_allowlist;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_VALUES: usize = 5_000_000;
pub const MAX_PARTS: usize = 10_000;
pub const ALLOWLIST_VERSION: u32 = 4;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Value {
    pub location: String,
    pub kind: String,
    pub value: Option<String>,
    pub content: bool,
    pub warning: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Removal {
    pub location: String,
    pub reason: String,
    pub value: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub expected_sha256: String,
    pub value: String,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Report {
    pub allowlist_version: u32,
    pub inventory: Vec<Value>,
    pub removals: Vec<Removal>,
    pub warnings: Vec<String>,
    pub expanded_bytes: u64,
    pub parts: usize,
    pub source_parts: usize,
}
impl Report {
    fn remove(&mut self, location: &str, reason: &str, value: String) {
        self.removals.push(Removal {
            location: location.into(),
            reason: reason.into(),
            value,
        });
        self.warnings.push(format!("{location}: {reason}"));
    }
    fn add(
        &mut self,
        location: String,
        kind: &str,
        value: Option<String>,
        content: bool,
        warning: bool,
    ) -> Result<()> {
        ensure!(
            self.inventory.len() < MAX_VALUES,
            "OFFICE_INVENTORY_LIMIT_EXCEEDED at {location}: {} / {MAX_VALUES}",
            self.inventory.len() + 1
        );
        self.inventory.push(Value {
            location,
            kind: kind.into(),
            value,
            content,
            warning,
        });
        Ok(())
    }
    pub fn text(&self) -> String {
        self.inventory
            .iter()
            .filter(|v| v.content)
            .filter_map(|v| v.value.as_deref())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
#[derive(Clone, Debug)]
struct Node {
    ns: String,
    local: String,
    attrs: BTreeMap<(String, String), String>,
    children: Vec<Node>,
    text: String,
    ignorable: std::sync::Arc<BTreeSet<String>>,
}
impl Node {
    fn shape_id(&self) -> Option<&str> {
        if ![P, XDR, PIC].contains(&self.ns.as_str())
            || !words("sp pic grpSp graphicFrame", &self.local)
        {
            return None;
        }
        self.children
            .iter()
            .filter(|n| words("nvSpPr nvPicPr nvGrpSpPr nvGraphicFramePr", &n.local))
            .flat_map(|n| &n.children)
            .find(|n| n.local == "cNvPr")
            .and_then(|n| n.attr("id"))
    }
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .get(&(String::new(), key.into()))
            .map(String::as_str)
    }
    fn attr_ns(&self, ns: &str, key: &str) -> Option<&str> {
        self.attrs.get(&(ns.into(), key.into())).map(String::as_str)
    }
    fn string(&self) -> String {
        let mut s = self.text.clone();
        for c in &self.children {
            s.push_str(&c.string());
        }
        s
    }
}
fn namespace(ns: ResolveResult<'_>) -> Result<String> {
    Ok(match ns {
        ResolveResult::Bound(ns) => std::str::from_utf8(ns.as_ref())?.into(),
        ResolveResult::Unbound => String::new(),
        ResolveResult::Unknown(p) => bail!("unbound XML prefix {}", String::from_utf8_lossy(&p)),
    })
}
fn parse(bytes: &[u8], part: &str) -> Result<Node> {
    // quick-xml is pinned by S1927. RUSTSEC-2026-0194 affects NsReader's
    // duplicate-attribute scan. Bound tags/attributes with the plain reader
    // before namespace resolution can reach that quadratic path.
    ensure!(
        !bytes.windows(9).any(|w| w == b"<!DOCTYPE"),
        "DTD/entities forbidden at {part}"
    );
    let mut preflight = quick_xml::Reader::from_reader(bytes);
    let mut buf = Vec::new();
    loop {
        match preflight.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) => {
                ensure!(
                    e.len() <= 64 * 1024,
                    "OFFICE_XML_TAG_LIMIT_EXCEEDED at {part}: {} / 65536 bytes",
                    e.len()
                );
                let mut seen = BTreeSet::new();
                let mut count = 0;
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    count += 1;
                    ensure!(
                        count <= 128,
                        "OFFICE_XML_ATTRIBUTE_LIMIT_EXCEEDED at {part}: {count} / 128"
                    );
                    ensure!(
                        seen.insert(attr.key.as_ref().to_vec()),
                        "duplicate XML attribute at {part}"
                    );
                }
            }
            Event::DocType(_) => bail!("DTD/entities forbidden at {part}"),
            Event::Eof => break,
            _ => (),
        }
        buf.clear();
    }
    let mut reader = NsReader::from_str(std::str::from_utf8(bytes)?);
    let mut stack: Vec<Node> = vec![];
    let mut root = None;
    let mut count = 0;
    fn attach(n: Node, stack: &mut [Node], root: &mut Option<Node>) -> Result<()> {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(n);
        } else {
            ensure!(root.replace(n).is_none(), "multiple XML roots");
        }
        Ok(())
    }
    loop {
        let ev = reader.read_event()?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                ensure!(
                    count <= MAX_VALUES && stack.len() < 128,
                    "OFFICE_XML_LIMIT_EXCEEDED at {part}: {count} nodes / {} depth",
                    stack.len()
                );
                let (ns, local) = reader.resolve_element(e.name());
                let mut n = Node {
                    ns: namespace(ns)?,
                    local: std::str::from_utf8(local.as_ref())?.into(),
                    attrs: BTreeMap::new(),
                    children: vec![],
                    text: String::new(),
                    ignorable: stack
                        .last()
                        .map(|n| n.ignorable.clone())
                        .unwrap_or_default(),
                };
                for a in e.attributes() {
                    let a = a?;
                    if a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:") {
                        continue;
                    }
                    let (ns, local) = reader.resolve_attribute(a.key);
                    let key = (namespace(ns)?, std::str::from_utf8(local.as_ref())?.into());
                    ensure!(
                        n.attrs
                            .insert(
                                key,
                                a.decode_and_unescape_value(reader.decoder())?.into_owned()
                            )
                            .is_none(),
                        "duplicate expanded attribute at {part}"
                    );
                }
                if n.ns == MC && n.local == "Choice" {
                    let requires = n
                        .attr("Requires")
                        .context("mc:Choice lacks Requires")?
                        .to_owned();
                    let mut namespaces = Vec::new();
                    for prefix in requires.split_ascii_whitespace() {
                        let qname = format!("{prefix}:_");
                        let (ns, _) =
                            reader.resolve_element(quick_xml::name::QName(qname.as_bytes()));
                        namespaces.push(namespace(ns)?);
                    }
                    ensure!(!namespaces.is_empty(), "empty mc:Choice Requires at {part}");
                    n.attrs
                        .insert((String::new(), "Requires".into()), namespaces.join(" "));
                }
                if let Some(value) = n.attrs.get(&(MC.into(), "Ignorable".into())) {
                    for prefix in value.split_ascii_whitespace() {
                        let qname = format!("{prefix}:_");
                        let (ns, _) =
                            reader.resolve_element(quick_xml::name::QName(qname.as_bytes()));
                        let ns = namespace(ns)?;
                        ensure!(
                            ns.starts_with("http://schemas.microsoft.com/office/"),
                            "unsupported mc:Ignorable namespace at {part}"
                        );
                        std::sync::Arc::make_mut(&mut n.ignorable).insert(ns);
                    }
                }
                if matches!(ev, Event::Empty(_)) {
                    attach(n, &mut stack, &mut root)?;
                } else {
                    stack.push(n);
                }
            }
            Event::End(_) => {
                let n = stack.pop().context("unbalanced XML")?;
                attach(n, &mut stack, &mut root)?;
            }
            Event::Text(t) => {
                let s = quick_xml::escape::unescape(&t.decode()?)?.into_owned();
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&s);
                    ensure!(
                        n.text.len() <= 1024 * 1024,
                        "OFFICE_XML_TEXT_LIMIT_EXCEEDED at {part}: {} / 1048576 bytes",
                        n.text.len()
                    );
                } else {
                    ensure!(s.trim().is_empty(), "XML text outside root");
                }
            }
            Event::CData(t) => stack
                .last_mut()
                .context("CDATA outside root")?
                .text
                .push_str(&t.decode()?),
            Event::DocType(_) => bail!("DTD/entities forbidden at {part}"),
            Event::GeneralRef(reference) => {
                let value = if let Some(ch) = reference.resolve_char_ref()? {
                    ensure!(
                        ch == '\t' || ch == '\n' || ch == '\r' || ch >= ' ',
                        "invalid XML character at {part}"
                    );
                    ch.to_string()
                } else {
                    let decoded = reference.decode()?;
                    match decoded.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "apos" => "'",
                        "quot" => "\"",
                        _ => bail!("DTD/entities forbidden at {part}"),
                    }
                    .into()
                };
                stack
                    .last_mut()
                    .context("reference outside root")?
                    .text
                    .push_str(&value);
            }
            Event::PI(_) => bail!("XML processing instruction forbidden at {part}"),
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) => (),
        }
    }
    ensure!(stack.is_empty(), "incomplete XML at {part}");
    root.context("missing XML root")
}
fn serialize(n: &Node) -> Result<Vec<u8>> {
    use quick_xml::{
        events::{BytesEnd, BytesStart, BytesText},
        Writer,
    };
    fn collect(n: &Node, s: &mut BTreeSet<String>) {
        if !n.ns.is_empty() {
            s.insert(n.ns.clone());
        }
        for (ns, _) in n.attrs.keys() {
            if !ns.is_empty() {
                s.insert(ns.clone());
            }
        }
        for c in &n.children {
            collect(c, s);
        }
    }
    let mut ns = BTreeSet::new();
    collect(n, &mut ns);
    let prefixes: BTreeMap<String, String> = ns
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let p = if s == XML {
                "xml".into()
            } else {
                format!("n{i}")
            };
            (s, p)
        })
        .collect();
    fn qname(ns: &str, k: &str, p: &BTreeMap<String, String>) -> String {
        if ns.is_empty() {
            k.into()
        } else {
            format!("{}:{k}", p[ns])
        }
    }
    fn write(
        n: &Node,
        p: &BTreeMap<String, String>,
        w: &mut Writer<Vec<u8>>,
        root: bool,
    ) -> Result<()> {
        let name = qname(&n.ns, &n.local, p);
        let mut start = BytesStart::new(&name);
        let mut attrs: Vec<_> = n
            .attrs
            .iter()
            .map(|((ns, k), v)| (qname(ns, k, p), v.clone()))
            .collect();
        if root {
            for (ns, pr) in p {
                if ns != XML {
                    attrs.push((format!("xmlns:{pr}"), ns.clone()));
                }
            }
        }
        for (k, v) in &attrs {
            let escaped = quick_xml::escape::escape(v)
                .replace('\n', "&#10;")
                .replace('\r', "&#13;")
                .replace('\t', "&#9;");
            start.push_attribute(quick_xml::events::attributes::Attribute {
                key: quick_xml::name::QName(k.as_bytes()),
                value: std::borrow::Cow::Owned(escaped.into_bytes()),
            });
        }
        w.write_event(Event::Start(start))?;
        if !n.text.is_empty() {
            w.write_event(Event::Text(BytesText::new(&n.text)))?;
        }
        for c in &n.children {
            write(c, p, w, false)?;
        }
        w.write_event(Event::End(BytesEnd::new(&name)))?;
        Ok(())
    }
    let mut writer = Writer::new(Vec::new());
    write(n, &prefixes, &mut writer, true)?;
    Ok(writer.into_inner())
}
fn words(s: &str, k: &str) -> bool {
    s.split_ascii_whitespace().any(|x| x == k)
}

// Version 3: each entry is an element plus its permitted unqualified (or
// Word-qualified) attributes. This deliberately refuses unimplemented layout
// constructs; a namespace match by itself never permits retention.
fn rule(ns: &str, el: &str) -> Option<&'static str> {
    if let Some(attrs) = diagram_allowlist::rule(ns, el) {
        return Some(attrs);
    }
    if let Some(attrs) = chart_allowlist::rule(ns, el) {
        return Some(attrs);
    }
    if ns==X14 && words("dataValidations dataValidation conditionalFormatting cfRule colorScale dataBar iconSet cfvo",el) {return rule(S,el);}
    if ns == X14 && words("conditionalFormattings formula1 formula2", el) {
        return Some("");
    }
    if ns == XM && words("f sqref", el) {
        return Some("");
    }
    Some(match (ns,el) {
        (XDR,"graphicFrame"|"nvGraphicFramePr"|"cNvGraphicFramePr"|"xfrm")=>"macro",
        (S,"chartsheet")=>"",

        (W,"rStyle"|"tblStyleColBandSize"|"tblStyleRowBandSize")=>"val",
        (W,"separator"|"continuationSeparator")=>"",
        (P,"notesStyle")=>"",(P,"nvPr")=>"isPhoto userDrawn",
        (P,"bg")=>"bwMode",(P,"bgPr")=>"shadeToTitle",(P,"bgRef")=>"idx",
        (A,"spPr")=>"bwMode",(A,"style")=>"",
        (A,"custGeom"|"gdLst"|"ahLst"|"cxnLst"|"pathLst"|"close")=>"",
        (A,"ahXY")=>"gdRefX minX maxX gdRefY minY maxY",
        (A,"ahPolar")=>"gdRefR minR maxR gdRefAng minAng maxAng",
        (A,"cxn")=>"ang",(A,"pos"|"pt")=>"x y",
        (A,"rect")=>"l t r b",(A,"moveTo"|"lnTo"|"cubicBezTo"|"quadBezTo")=>"",
        (A,"arcTo")=>"wR hR stAng swAng",
        (S,"filterColumn")=>"colId hiddenButton showButton",
        (S,"filters")=>"blank calendarType",(S,"filter")=>"val",
        (S,"customFilters")=>"and",(S,"customFilter")=>"operator val",
        (S,"top10")=>"top percent val filterVal",
        (S,"dynamicFilter")=>"type val maxVal valIso maxValIso",
        (S,"colorFilter")=>"dxfId cellColor",(S,"iconFilter")=>"iconSet iconId",
        (S,"dateGroupItem")=>"year month day hour minute second dateTimeGrouping",
        (A,"sym")=>"typeface panose pitchFamily charset",
        (S,"dxf")=>"",(S,"pane")=>"xSplit ySplit topLeftCell activePane state",

        (W,"tab")=>"val leader pos",
        (W,"zoom")=>"val percent",
        (S,"extLst")=>"",(S,"ext")=>"uri",
        (W,"footnoteReference"|"endnoteReference")=>"id customMarkFollows",
        (A,"masterClrMapping")=>"",
        (WP,"inline")=>"distT distB distL distR",
        (WP,"anchor")=>"distT distB distL distR simplePos relativeHeight behindDoc locked layoutInCell hidden allowOverlap",
        (WP,"docPr")=>"id name descr hidden title",(WP,"extent")=>"cx cy",(WP,"effectExtent")=>"l t r b",
        (WP,"simplePos"|"start"|"lineTo")=>"x y",(WP,"positionH"|"positionV")=>"relativeFrom",
        (WP,"align"|"posOffset"|"cNvGraphicFramePr"|"wrapNone"|"wrapTopAndBottom")=>"",
        (WP,"wrapSquare"|"wrapTight"|"wrapThrough")=>"wrapText",(WP,"wrapPolygon")=>"edited",
        (PIC,"pic"|"nvPicPr"|"cNvPicPr"|"blipFill"|"spPr")=>"",
        (PIC,"cNvPr")=>"id name descr hidden title",
        (XDR,"wsDr"|"oneCellAnchor"|"absoluteAnchor"|"from"|"to"|"col"|"row"|"colOff"|"rowOff"|"pic"|"nvPicPr"|"cNvPicPr"|"blipFill"|"spPr"|"sp"|"nvSpPr"|"cNvSpPr"|"txBody")=>"",
        (XDR,"cNvPr")=>"id name descr hidden title",(XDR,"twoCellAnchor")=>"editAs",
        (XDR,"pos")=>"x y",(XDR,"ext")=>"cx cy",(XDR,"clientData")=>"fLocksWithSheet fPrintsWithSheet",
        (P14,"sectionLst"|"sldIdLst")=>"", (P14,"section")=>"name id",(P14,"sldId")=>"id",
        (P,"extLst"|"pic"|"nvPicPr"|"cNvPicPr"|"blipFill"|"grpSp")=>"", (P,"ext")=>"uri",
        (A,"graphic"|"stretch"|"fillRect"|"blipFill")=>"", (A,"graphicData")=>"uri",
        (A,"blip")=>"cstate",(A,"srcRect")=>"l t r b",(A,"tile")=>"tx ty sx sy flip algn",
        (A,"graphicFrameLocks"|"picLocks"|"spLocks"|"grpSpLocks")=>"noGrp noSelect noRot noChangeAspect noMove noResize noEditPoints noAdjustHandles noChangeArrowheads noChangeShapeType noCrop",
        (CT,"Types")|(REL,"Relationships")=>"",
        (CT,"Default")=>"Extension ContentType",(CT,"Override")=>"PartName ContentType",
        (REL,"Relationship")=>"Id Type Target TargetMode",
        (W,"document"|"body"|"p"|"pPr"|"r"|"rPr"|"t"|"tbl"|"tblPr"|"tblGrid"|"tr"|"trPr"|"tc"|"tcPr"|"hdr"|"ftr"|"styles"|"settings"|"numbering"|"footnotes"|"endnotes"|"drawing"|"docDefaults"|"rPrDefault"|"pPrDefault")=>"",
        (W,"hyperlink")=>"anchor history tooltip docLocation tgtFrame",
        (W,"bookmarkStart")=>"id name colFirst colLast",(W,"bookmarkEnd")=>"id",
        (W,"footnote"|"endnote")=>"id type",(W,"style")=>"type styleId default customStyle",
        (W,"headerReference"|"footerReference")=>"type",
        (W,"rFonts")=>"ascii hAnsi eastAsia cs asciiTheme hAnsiTheme eastAsiaTheme cstheme hint",
        (W,"lang")=>"val eastAsia bidi",(W,"spacing")=>"val before after line lineRule beforeAutospacing afterAutospacing beforeLines afterLines",
        (W,"ind")=>"left right firstLine hanging start end leftChars rightChars firstLineChars hangingChars",
        (W,"sectPr")=>"", (W,"pgSz")=>"w h orient code",(W,"pgMar")=>"top right bottom left header footer gutter",
        (W,"cols")=>"equalWidth space num sep",(W,"col")=>"w space",(W,"docGrid")=>"type linePitch charSpace",
        (W,"tblW"|"tcW")=>"w type",(W,"gridCol")=>"w",(W,"br")=>"type clear",
        (W,"b"|"i"|"u"|"strike"|"dstrike"|"caps"|"smallCaps"|"sz"|"szCs"|"highlight"|"vertAlign"|"jc"|"keepNext"|"keepLines"|"pageBreakBefore"|"outlineLvl"|"contextualSpacing"|"vanish"|"webHidden"|"noProof"|"widowControl"|"vAlign"|"gridSpan"|"vMerge"|"textDirection"|"name"|"basedOn"|"next"|"link"|"uiPriority"|"qFormat"|"pStyle"|"numId"|"ilvl"|"start"|"numFmt"|"lvlText"|"lvlJc"|"abstractNumId"|"multiLevelType"|"tblLayout"|"defaultTabStop"|"characterSpacingControl")=>"val",
        (W,"color")=>"val themeColor themeTint themeShade",(W,"shd")=>"val color fill themeColor themeFill themeFillTint themeFillShade",
        (W,"cr"|"lastRenderedPageBreak"|"softHyphen"|"noBreakHyphen"|"tblBorders"|"tcBorders"|"cantSplit"|"tblHeader"|"titlePg"|"evenAndOddHeaders")=>"",
        (W,"top"|"left"|"bottom"|"right"|"insideH"|"insideV")=>"val sz space color themeColor themeTint themeShade shadow frame w type",
        (W,"num")=>"numId",(W,"abstractNum")=>"abstractNumId",(W,"lvl")=>"ilvl tplc tentative",(W,"lvlOverride")=>"ilvl",
        (S,"workbook")=>"conformance",(S,"worksheet"|"sheets"|"sheetData"|"si"|"r"|"rPr"|"t"|"v"|"is"|"bookViews"|"sheetViews"|"definedNames"|"styleSheet"|"colors"|"indexedColors")=>"",
        (S,"sheet")=>"name sheetId state",(S,"c")=>"r s t",(S,"row")=>"r spans s customFormat ht hidden customHeight outlineLevel collapsed thickTop thickBot",
        (S,"col")=>"min max width style hidden bestFit customWidth outlineLevel collapsed",(S,"cols")=>"",
        (S,"sst")=>"count uniqueCount",(S,"dimension"|"mergeCell"|"autoFilter")=>"ref",(S,"mergeCells")=>"count",
        (S,"sheetFormatPr")=>"baseColWidth defaultColWidth defaultRowHeight customHeight zeroHeight thickTop thickBottom outlineLevelRow outlineLevelCol",
        (S,"sheetView")=>"zoomToFit windowProtection showFormulas showGridLines showRowColHeaders showZeros rightToLeft tabSelected showRuler showOutlineSymbols defaultGridColor view topLeftCell colorId zoomScale zoomScaleNormal zoomScaleSheetLayoutView zoomScalePageLayoutView workbookViewId",
        (S,"workbookView")=>"visibility minimized showHorizontalScroll showVerticalScroll showSheetTabs xWindow yWindow windowWidth windowHeight tabRatio firstSheet activeTab autoFilterDateGrouping",
        (S,"workbookPr")=>"date1904 showObjects filterPrivacy defaultThemeVersion",
        (S,"selection")=>"pane activeCell activeCellId sqref",(S,"sheetPr")=>"syncHorizontal syncVertical syncRef transitionEvaluation transitionEntry published filterMode enableFormatConditionsCalculation",
        (S,"outlinePr")=>"applyStyles summaryBelow summaryRight showOutlineSymbols",(S,"pageSetUpPr")=>"autoPageBreaks fitToPage",
        (S,"pageMargins")=>"left right top bottom header footer",(S,"pageSetup")=>"paperSize scale firstPageNumber fitToWidth fitToHeight pageOrder orientation usePrinterDefaults blackAndWhite draft cellComments useFirstPageNumber errors horizontalDpi verticalDpi copies",
        (S,"headerFooter")=>"differentOddEven differentFirst scaleWithDoc alignWithMargins",
        (S,"oddHeader"|"oddFooter"|"evenHeader"|"evenFooter"|"firstHeader"|"firstFooter")=>"",
        (S,"printOptions")=>"horizontalCentered verticalCentered headings gridLines gridLinesSet",
        (S,"fonts"|"fills"|"borders"|"cellXfs"|"cellStyleXfs"|"cellStyles"|"numFmts"|"dxfs"|"tableStyles")=>"count defaultTableStyle defaultPivotStyle",
        (S,"font"|"fill")=>"",
        (S,"xf")=>"numFmtId fontId fillId borderId xfId applyNumberFormat applyFont applyFill applyBorder applyAlignment applyProtection pivotButton quotePrefix",
        (S,"alignment")=>"horizontal vertical textRotation wrapText shrinkToFit indent relativeIndent justifyLastLine readingOrder",
        (S,"protection")=>"locked hidden",(S,"cellStyle")=>"name xfId builtinId iLevel hidden customBuiltin",
        (S,"numFmt")=>"numFmtId formatCode",(S,"name"|"sz"|"family"|"charset"|"scheme"|"b"|"i"|"u"|"strike"|"rFont")=>"val",
        (S,"patternFill")=>"patternType",(S,"fgColor"|"bgColor"|"color"|"rgbColor")=>"rgb indexed auto theme tint",
        (S,"border")=>"diagonalUp diagonalDown outline",(S,"left"|"right"|"top"|"bottom"|"diagonal")=>"style",
        (S,"table")=>"id name displayName ref headerRowCount totalsRowCount totalsRowShown",
        (S,"tableColumns")=>"count",(S,"tableColumn")=>"id name totalsRowLabel",
        (S,"tableStyleInfo")=>"name showFirstColumn showLastColumn showRowStripes showColumnStripes",
        (S,"tableParts"|"dataValidations")=>"count disablePrompts xWindow yWindow",(S,"tablePart"|"drawing")=>"",
        (S,"hyperlinks")=>"",(S,"hyperlink")=>"ref location tooltip display",
        (S,"dataValidation")=>"type errorStyle imeMode operator allowBlank showDropDown showInputMessage showErrorMessage errorTitle error promptTitle prompt sqref",
        (S,"conditionalFormatting")=>"sqref pivot",(S,"cfRule")=>"type dxfId priority stopIfTrue aboveAverage percent bottom operator text timePeriod rank stdDev equalAverage",
        (S,"colorScale"|"dataBar"|"iconSet")=>"showValue minLength maxLength iconSet percent reverse",
        (S,"cfvo")=>"type val gte",
        (P,"presentation")=>"serverZoom firstSlideNum showSpecialPls rtl removePersonalInfo compatMode strictFirstAndLastChars embedTrueTypeFonts saveSubsetFonts autoCompressPictures bookmarkIdSeed conformance",
        (P,"sld")=>"show showMasterSp showMasterPhAnim",(P,"cSld")=>"name",
        (P,"sldIdLst"|"sldMasterIdLst"|"notesMasterIdLst"|"handoutMasterIdLst"|"spTree"|"nvGrpSpPr"|"cNvGrpSpPr"|"grpSpPr"|"sp"|"nvSpPr"|"spPr"|"txBody"|"style"|"notes"|"notesMaster"|"handoutMaster"|"clrMapOvr"|"masterClrMapping"|"sldLayoutIdLst"|"txStyles"|"titleStyle"|"bodyStyle"|"otherStyle"|"defaultTextStyle")=>"",
        (P,"sldId"|"sldMasterId"|"sldLayoutId")=>"id",(P,"notesMasterId"|"handoutMasterId")=>"",
        (P,"sldSz")=>"cx cy type",(P,"notesSz")=>"cx cy",
        (P,"sldLayout")=>"type preserve matchingName showMasterSp showMasterPhAnim userDrawn",(P,"sldMaster")=>"preserve",
        (P,"hf")=>"sldNum hdr ftr dt",(P,"cNvPr")=>"id name descr hidden title",(P,"cNvSpPr")=>"txBox",
        (P,"ph")=>"type orient sz idx hasCustomPrompt",(P,"clrMap")=>"bg1 tx1 bg2 tx2 accent1 accent2 accent3 accent4 accent5 accent6 hlink folHlink",
        (A,"t"|"p"|"r"|"br"|"txBody"|"lstStyle"|"solidFill"|"noFill"|"effectLst"|"themeElements"|"objectDefaults"|"extraClrSchemeLst"|"fillStyleLst"|"lnStyleLst"|"effectStyleLst"|"effectStyle"|"bgFillStyleLst")=>"",
        (A,"theme"|"clrScheme"|"fontScheme"|"fmtScheme")=>"name",
        (A,"dk1"|"lt1"|"dk2"|"lt2"|"accent1"|"accent2"|"accent3"|"accent4"|"accent5"|"accent6"|"hlink"|"folHlink"|"majorFont"|"minorFont")=>"",
        (A,"srgbClr"|"schemeClr"|"lumMod"|"lumOff"|"alpha"|"tint"|"shade")=>"val",(A,"sysClr")=>"val lastClr",
        (A,"off"|"chOff")=>"x y",(A,"ext")=>"cx cy uri",(A,"chExt")=>"cx cy",(A,"xfrm")=>"rot flipH flipV",
        (A,"prstGeom")=>"prst",(A,"avLst")=>"",(A,"gd")=>"name fmla",
        (A,"bodyPr")=>"rot spcFirstLastPara vertOverflow horzOverflow vert wrap lIns tIns rIns bIns numCol spcCol rtlCol fromWord anchor anchorCtr forceAA compatLnSpc upright",
        (A,"rPr"|"defRPr"|"endParaRPr")=>"kumimoji lang altLang sz b i u strike kern cap spc normalizeH baseline noProof dirty err smtClean smtId bmk",
        (A,"pPr"|"lvl1pPr"|"lvl2pPr"|"lvl3pPr"|"lvl4pPr"|"lvl5pPr"|"lvl6pPr"|"lvl7pPr"|"lvl8pPr"|"lvl9pPr")=>"marL marR lvl indent algn defTabSz rtl eaLnBrk fontAlgn latinLnBrk hangingPunct",
        (A,"latin"|"ea"|"cs"|"font")=>"typeface panose pitchFamily charset script",
        (A,"spAutoFit"|"noAutofit"|"buNone")=>"",(A,"normAutofit")=>"fontScale lnSpcReduction",
        (A,"buChar")=>"char",(A,"buFont")=>"typeface panose pitchFamily charset",(A,"buAutoNum")=>"type startAt",
        (A,"lnRef"|"fillRef"|"effectRef"|"fontRef")=>"idx",(A,"ln")=>"w cap cmpd algn",
        (A,"prstDash")=>"val",(A,"headEnd"|"tailEnd")=>"type w len",
        (A,"hlinkClick"|"hlinkHover")=>"tooltip tgtFrame history highlightClick endSnd",
        _=>return style_allowlist::rule(ns,el).or_else(|| layout_allowlist::rule(ns,el)),
    })
}

fn path_key(path: &str) -> Result<String> {
    ensure!(path.is_ascii()&&!path.bytes().any(|b|b.is_ascii_control()),"unsupported non-ASCII/control Office part name; canonical collision safety cannot be proved");
    ensure!(
        !path.is_empty() && !path.starts_with('/') && !path.contains(['\\', ':', '\0', '?', '#']),
        "unsafe Office part {path}"
    );
    let mut decoded = Vec::new();
    let b = path.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            ensure!(i + 2 < b.len(), "invalid URI escape {path}");
            let h = std::str::from_utf8(&b[i + 1..i + 3])?;
            decoded.push(u8::from_str_radix(h, 16)?);
            i += 3;
        } else {
            decoded.push(b[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(decoded)?;
    ensure!(
        decoded == path,
        "escaped Office part names are unsupported: {path}"
    );
    ensure!(
        path.split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".."),
        "unsafe Office part {path}"
    );
    Ok(path.to_lowercase())
}
fn web_target(target: &str) -> bool {
    target
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
        || target
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
}
pub(crate) fn resolve(owner: &str, target: &str) -> Result<String> {
    ensure!(
        !target.contains(['\\', ':', '\0', '?', '%']),
        "unsupported relationship URI"
    );
    let target = target.split('#').next().unwrap_or_default();
    let mut parts: Vec<&str> = if target.starts_with('/') {
        vec![]
    } else {
        owner
            .rsplit_once('/')
            .map_or(vec![], |(p, _)| p.split('/').collect())
    };
    for p in target.trim_start_matches('/').split('/') {
        match p {
            "" | "." => (),
            ".." => {
                ensure!(parts.pop().is_some(), "relationship escapes package root");
            }
            _ => parts.push(p),
        }
    }
    let path = parts.join("/");
    path_key(&path)?;
    Ok(path)
}
fn owner(rels: &str) -> Result<String> {
    if rels == "_rels/.rels" {
        return Ok(String::new());
    }
    let (dir, file) = rels
        .rsplit_once("/_rels/")
        .context("invalid relationships part path")?;
    Ok(format!(
        "{dir}/{}",
        file.strip_suffix(".rels")
            .context("invalid relationships suffix")?
    ))
}
fn removable(path: &str) -> bool {
    if path.ends_with(".rels") {
        if let Ok(parent) = owner(path) {
            if !parent.is_empty() {
                return removable(&parent);
            }
        }
    }
    if ["word/webSettings.xml", "ppt/viewProps.xml"].contains(&path) {
        return true;
    }
    path.starts_with("docProps/")
        || path.to_ascii_lowercase().starts_with("customxml/")
        || path.to_ascii_lowercase().starts_with("[trash]/")
        || path.starts_with("docMetadata/")
        || path.contains("/comments")
        || path.contains("/threadedComments")
        || path.contains("/persons/")
        || path.ends_with("/people.xml")
        || path.ends_with("/commentAuthors.xml")
        || path.contains("/printerSettings/")
        || path.contains("/pivotCache/")
        || path.contains("/pivotTables/")
        || path.contains("/revisions/")
        || path.contains("/webextensions/")
        || path.contains("/tags/")
        || path.ends_with("/revisionInfo.xml")
        || path.contains("/changesInfos/")
        || path.contains("/glossary/")
        || (path.starts_with("xl/drawings/") && path.ends_with(".vml"))
        || path.contains("/fonts/")
        || path.contains("/ink/")
}
fn root_for(path: &str) -> Option<(&'static str, &'static str)> {
    Some(if path == "[Content_Types].xml" {
        (CT, "Types")
    } else if path.ends_with(".rels") {
        (REL, "Relationships")
    } else if path == "word/document.xml" {
        (W, "document")
    } else if ["word/styles.xml", "word/stylesWithEffects.xml"].contains(&path) {
        (W, "styles")
    } else if path == "word/fontTable.xml" {
        (W, "fonts")
    } else if path == "word/settings.xml" {
        (W, "settings")
    } else if path == "word/numbering.xml" {
        (W, "numbering")
    } else if path == "word/footnotes.xml" {
        (W, "footnotes")
    } else if path == "word/endnotes.xml" {
        (W, "endnotes")
    } else if path.starts_with("word/header") && path.ends_with(".xml") {
        (W, "hdr")
    } else if path.starts_with("word/footer") && path.ends_with(".xml") {
        (W, "ftr")
    } else if path == "xl/workbook.xml" {
        (S, "workbook")
    } else if path.contains("/diagrams/") && path.ends_with(".xml") {
        (
            if path.rsplit('/').next()?.starts_with("drawing") {
                graphics::DSP
            } else {
                graphics::DGM
            },
            if path.rsplit('/').next()?.starts_with("drawing") {
                "drawing"
            } else if path.rsplit('/').next()?.starts_with("data") {
                "dataModel"
            } else if path.rsplit('/').next()?.starts_with("layout") {
                "layoutDef"
            } else if path.rsplit('/').next()?.starts_with("quickStyle") {
                "styleDef"
            } else if path.rsplit('/').next()?.starts_with("colors") {
                "colorsDef"
            } else {
                return None;
            },
        )
    } else if path.contains("/charts/") && path.ends_with(".xml") {
        (
            if path.rsplit('/').next().unwrap_or("").starts_with("style") {
                graphics::CS
            } else if path.rsplit('/').next().unwrap_or("").starts_with("colors") {
                graphics::CS
            } else {
                graphics::C
            },
            if path.rsplit('/').next().unwrap_or("").starts_with("style") {
                "chartStyle"
            } else if path.rsplit('/').next().unwrap_or("").starts_with("colors") {
                "colorStyle"
            } else {
                "chartSpace"
            },
        )
    } else if path.starts_with("xl/chartsheets/") && path.ends_with(".xml") {
        (S, "chartsheet")
    } else if path.starts_with("xl/worksheets/") && path.ends_with(".xml") {
        (S, "worksheet")
    } else if path == "xl/sharedStrings.xml" {
        (S, "sst")
    } else if path == "xl/styles.xml" {
        (S, "styleSheet")
    } else if path.starts_with("xl/tables/") && path.ends_with(".xml") {
        (S, "table")
    } else if path == "ppt/tableStyles.xml" {
        (A, "tblStyleLst")
    } else if path == "ppt/presProps.xml" {
        (P, "presentationPr")
    } else if path == "ppt/presentation.xml" {
        (P, "presentation")
    } else if path.starts_with("ppt/slides/") && path.ends_with(".xml") {
        (P, "sld")
    } else if path.starts_with("ppt/slideMasters/") && path.ends_with(".xml") {
        (P, "sldMaster")
    } else if path.starts_with("ppt/slideLayouts/") && path.ends_with(".xml") {
        (P, "sldLayout")
    } else if path.starts_with("ppt/notesSlides/") && path.ends_with(".xml") {
        (P, "notes")
    } else if path.starts_with("ppt/notesMasters/") && path.ends_with(".xml") {
        (P, "notesMaster")
    } else if path.starts_with("ppt/handoutMasters/") && path.ends_with(".xml") {
        (P, "handoutMaster")
    } else if path.starts_with("xl/drawings/") && path.ends_with(".xml") {
        (XDR, "wsDr")
    } else if path.contains("/theme/") && path.ends_with(".xml") {
        (A, "theme")
    } else {
        return None;
    })
}
// OOXML numeric lexical values only: no expressions, whitespace or nonfinite values.
fn plain_number(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
        && value.parse::<f64>().is_ok_and(f64::is_finite)
}
fn formula(n: &Node) -> bool {
    ([S, X14].contains(&n.ns.as_str())
        && n.local == "cfvo"
        && (n.attr("type") == Some("formula") || n.attr("val").is_some_and(|v| !plain_number(v))))
        || ([S, X14].contains(&n.ns.as_str())
            && words(
                "f formula formula1 formula2 calculatedColumnFormula totalsRowFormula",
                &n.local,
            ))
        || (n.ns == "http://schemas.microsoft.com/office/excel/2006/main" && n.local == "f")
        || n.children.iter().any(formula)
}
fn cell_ref(value: &str) -> Result<(i64, i64)> {
    let split = value
        .find(|c: char| c.is_ascii_digit())
        .context("invalid cell reference")?;
    let (col, row) = value.split_at(split);
    ensure!(
        !col.is_empty() && col.bytes().all(|c| c.is_ascii_uppercase()),
        "invalid cell reference"
    );
    let mut column = 0i64;
    for c in col.bytes() {
        column = column
            .checked_mul(26)
            .and_then(|x| x.checked_add((c - b'A' + 1) as i64))
            .context("cell reference overflow")?;
    }
    let row: i64 = row.parse()?;
    ensure!(
        (1..=16384).contains(&column) && (1..=1048576).contains(&row),
        "cell reference outside Excel limits"
    );
    Ok((column, row))
}
fn column_name(mut col: i64) -> String {
    let mut result = Vec::new();
    while col > 0 {
        col -= 1;
        result.push(b'A' + (col % 26) as u8);
        col /= 26;
    }
    result.reverse();
    String::from_utf8(result).expect("ASCII column")
}
fn shift_formula(value: &str, dc: i64, dr: i64) -> Result<String> {
    ensure!(value.is_ascii()&&!value.contains(['[',']','#','@']),"unsupported shared-formula dependency; recalculate/save without shared formulas, or exclude");
    let b = value.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' || b[i] == b'\'' {
            let quote = b[i];
            out.push(quote as char);
            i += 1;
            let mut closed = false;
            while i < b.len() {
                out.push(b[i] as char);
                if b[i] == quote {
                    if b.get(i + 1) == Some(&quote) {
                        out.push(quote as char);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            ensure!(closed, "unterminated shared-formula string");
            continue;
        }
        let start = i;
        let absolute_col = b[i] == b'$';
        if absolute_col {
            i += 1;
        }
        let col_start = i;
        while i < b.len() && b[i].is_ascii_uppercase() {
            i += 1;
        }
        let col_end = i;
        let absolute_row = b.get(i) == Some(&b'$');
        if absolute_row {
            i += 1;
        }
        let row_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let boundary = start == 0
            || !b[start - 1].is_ascii_alphanumeric()
                && b[start - 1] != b'_'
                && b[start - 1] != b'.';
        let end_boundary = i == b.len()
            || !b[i].is_ascii_alphanumeric() && b[i] != b'_' && b[i] != b'(' && b[i] != b'.';
        if col_end > col_start && i > row_start && boundary && end_boundary {
            let token = format!("{}{}", &value[col_start..col_end], &value[row_start..i]);
            if let Ok((col, row)) = cell_ref(&token) {
                let col = if absolute_col { col } else { col + dc };
                let row = if absolute_row { row } else { row + dr };
                ensure!(
                    (1..=16384).contains(&col) && (1..=1048576).contains(&row),
                    "shared-formula expansion outside worksheet"
                );
                if absolute_col {
                    out.push('$');
                }
                out.push_str(&column_name(col));
                if absolute_row {
                    out.push('$');
                }
                out.push_str(&row.to_string());
                continue;
            }
        }
        i = start + 1;
        out.push(b[start] as char);
    }
    Ok(out)
}
fn inspect_formulas(root: &Node, part: &str, report: &mut Report) -> Result<()> {
    fn gather<'a>(n: &'a Node, cells: &mut Vec<&'a Node>) {
        if n.ns == S && n.local == "c" {
            cells.push(n);
        }
        for c in &n.children {
            gather(c, cells);
        }
    }
    let mut cells = Vec::new();
    gather(root, &mut cells);
    let mut shared = BTreeMap::new();
    let mut refs = BTreeSet::new();
    let mut cached = BTreeSet::new();
    for c in &cells {
        let reference = c.attr("r").context("cell lacks reference")?;
        let coords = cell_ref(reference)?;
        if c.children.iter().any(|v| {
            v.ns == S && v.local == "v" && (!v.text.is_empty() || c.attr("t") == Some("str"))
        }) {
            cached.insert(reference);
        }
        ensure!(
            refs.insert(reference),
            "duplicate worksheet cell reference at {part}"
        );
        if let Some(f) = c.children.iter().find(|f| f.ns == S && f.local == "f") {
            let ty = f.attr("t").unwrap_or("normal");
            ensure!(
                words("normal shared array", ty),
                "unsupported active formula construct at {part}/cell/{reference}"
            );
            if ty == "shared" && !f.text.is_empty() {
                ensure!(
                    shared
                        .insert(
                            f.attr("si")
                                .context("shared formula lacks group")?
                                .to_owned(),
                            (coords, f.text.clone())
                        )
                        .is_none(),
                    "ambiguous shared formula master at {part}"
                );
            }
        }
    }
    for c in cells {
        let reference = c.attr("r").context("cell lacks reference")?;
        if let Some(f) = c.children.iter().find(|f| f.ns == S && f.local == "f") {
            if f.attr("t") == Some("shared") {
                let (base, value) = shared
                    .get(f.attr("si").context("shared formula lacks group")?)
                    .context("shared formula lacks unique master")?;
                let at = cell_ref(reference)?;
                let expanded = shift_formula(value, at.0 - base.0, at.1 - base.1)?;
                report.remove(
                    &format!("{part}/cell/{reference}/formula"),
                    "expanded shared formula before values-only removal",
                    expanded,
                );
            } else if f.attr("t") == Some("array") {
                ensure!(!f.text.is_empty(), "array formula lacks expression");
                let range = f.attr("ref").context("array formula lacks range")?;
                let (first, last) = range.split_once(':').unwrap_or((range, range));
                let (a, b) = (cell_ref(first)?, cell_ref(last)?);
                ensure!(
                    b.0 >= a.0
                        && b.1 >= a.1
                        && (b.0 - a.0 + 1) * (b.1 - a.1 + 1) <= MAX_VALUES as i64,
                    "array formula range exceeds Office inventory limit at {part}"
                );
                for row in a.1..=b.1 {
                    for col in a.0..=b.0 {
                        let cell = format!("{}{row}", column_name(col));
                        ensure!(
                            cached.contains(cell.as_str()),
                            "uncached array result at {part}/cell/{cell}"
                        );
                        report.remove(
                            &format!("{part}/cell/{cell}/formula"),
                            "expanded array formula before values-only removal",
                            f.text.clone(),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
fn cleanse(n: &mut Node, part: &str, report: &mut Report) -> Result<()> {
    let mut resolved = Vec::new();
    for mut c in std::mem::take(&mut n.children) {
        if c.ns == MC && c.local == "AlternateContent" {
            ensure!(
                c.children
                    .iter()
                    .all(|b| b.ns == MC && words("Choice Fallback", &b.local)),
                "invalid AlternateContent at {part}"
            );
            let supported = c.children.iter().position(|b| {
                b.local == "Choice"
                    && b.attr("Requires").is_some_and(|r| {
                        r.split_ascii_whitespace().all(|ns| {
                            [
                                W,
                                S,
                                P,
                                A,
                                WP,
                                PIC,
                                XDR,
                                X14,
                                XM,
                                P14,
                                "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac",
                                "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main",
                            ]
                            .contains(&ns)
                        })
                    })
            });
            let selected =
                supported.or_else(|| c.children.iter().position(|b| b.local == "Fallback"));
            report.remove(
                part,
                "resolved AlternateContent; removed alternate branches",
                String::from_utf8(serialize(&c)?)?,
            );
            if let Some(selected) = selected {
                resolved.extend(std::mem::take(&mut c.children[selected].children));
            }
        } else {
            resolved.push(c);
        }
    }
    n.children = resolved;

    if !words("t v oddHeader oddFooter evenHeader evenFooter firstHeader firstFooter align posOffset col row colOff rowOff sqref tableStyleId formatCode",&n.local)&&n.text.trim().is_empty(){n.text.clear();}
    let mut old = Vec::new();
    for c in std::mem::take(&mut n.children) {
        if c.ns == W
            && c.local == "r"
            && c.children
                .iter()
                .any(|x| x.ns == W && words("fldChar instrText", &x.local))
        {
            let properties: Vec<_> = c
                .children
                .iter()
                .filter(|x| x.ns == W && x.local == "rPr")
                .cloned()
                .collect();
            for child in &c.children {
                if child.ns == W && child.local == "rPr" {
                    continue;
                }
                let mut run = Node {
                    ns: c.ns.clone(),
                    local: c.local.clone(),
                    attrs: c.attrs.clone(),
                    children: properties.clone(),
                    text: c.text.clone(),
                    ignorable: c.ignorable.clone(),
                };
                run.children.push(child.clone());
                old.push(run);
            }
        } else {
            old.push(c);
        }
    }
    let mut children = Vec::new();
    // Field instruction spans must be flattened across sibling runs. Keep only
    // result runs; nested fields are refused rather than guessed.
    let mut field: Option<(bool, bool)> = None;
    for mut c in old {
        if c.ns == W && c.local == "sdt" {
            ensure!(
                !field.is_some_and(|f| !f.0),
                "content control in Word field instruction region at {part}"
            );
            ensure!(
                c.children
                    .iter()
                    .all(|x| x.ns == W && words("sdtPr sdtEndPr sdtContent", &x.local))
                    && c.children
                        .iter()
                        .filter(|x| x.local == "sdtContent")
                        .count()
                        == 1,
                "unsupported Word content control without unique displayed content at {part}"
            );
            report.remove(
                part,
                "flattened Word content control; removed properties and data-store binding",
                String::from_utf8(serialize(&c)?)?,
            );
            let mut content = c.children.remove(
                c.children
                    .iter()
                    .position(|x| x.local == "sdtContent")
                    .unwrap(),
            );
            cleanse(&mut content, part, report)?;
            children.extend(content.children);
            continue;
        }
        if c.ns == A && c.local == "fld" {
            ensure!(
                c.children.iter().any(|x| x.ns == A && x.local == "t"),
                "Drawing field lacks displayed result at {part}"
            );
            report.remove(
                part,
                "flattened Drawing field to displayed result",
                String::from_utf8(serialize(&c)?)?,
            );
            ensure!(
                c.children
                    .iter()
                    .all(|x| x.ns == A && words("rPr pPr t", &x.local)),
                "unsupported Drawing field child at {part}"
            );
            c.children.retain(|x| x.local != "pPr");
            c.local = "r".into();
            c.attrs.clear();
        }
        if c.ns == A && words("hlinkClick hlinkHover", &c.local) && c.attr("action").is_some() {
            report.remove(
                part,
                "removed PowerPoint action",
                String::from_utf8(serialize(&c)?)?,
            );
            continue;
        }
        let marker = c
            .children
            .iter()
            .find(|x| x.ns == W && x.local == "fldChar")
            .map(|x| x.attr_ns(W, "fldCharType").unwrap_or("").to_owned());
        if let Some(marker) = marker {
            match marker.as_str() {
                "begin" => {
                    ensure!(field.is_none(), "nested Word field at {part}");
                    field = Some((false, false));
                }
                "separate" => {
                    let f = field.as_mut().context("field separator without begin")?;
                    f.0 = true;
                }
                "end" => {
                    ensure!(
                        field.take().is_some_and(|f| f.0 && f.1),
                        "Word field lacks safe displayed result at {part}"
                    );
                }
                _ => bail!("unknown Word field marker at {part}"),
            }
            report.remove(part, "flattened Word field marker", c.string());
            continue;
        }
        if let Some((result, seen)) = field.as_mut() {
            if !*result {
                report.remove(part, "removed Word field instruction", c.string());
                continue;
            }
            *seen |= !c.string().is_empty();
        }
        if n.ignorable.contains(&c.ns) && c.ns != P14 && c.ns != X14 && c.ns != graphics::DSP {
            report.remove(
                part,
                "removed schema-ignorable content",
                String::from_utf8(serialize(&c)?)?,
            );
            continue;
        }
        ensure!(
            !(c.ns == W
                && c.local == "r"
                && c.children
                    .iter()
                    .any(|x| x.ns == W && x.local == "instrText")),
            "Word field lacks safe displayed-result boundaries at {part}"
        );
        let drop=(n.ns==W&&n.local=="settings"&&c.ns=="http://schemas.openxmlformats.org/officeDocument/2006/math"&&c.local=="mathPr")||
            (c.ns==P&&words("clrMru printPr",&c.local))||
            (c.ns==W&&words("embedTrueTypeFonts embedSystemFonts saveSubsetFonts paperSrc savePreviewPicture printerSettings rsid aliases numIdMacAtCleanup formsDesign proofErr shapeDefaults hdrShapeDefaults doNotIncludeSubdocsInStats embedRegular embedBold embedItalic embedBoldItalic",&c.local))||
            (c.ns==S&&words("legacyDrawing legacyDrawingHF",&c.local))||
            (c.ns=="http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"&&c.local=="absPath")||
            (c.ns==S&&words("fileVersion fileRecoveryPr",&c.local))||
            (c.ns==W&&words("del moveFrom commentRangeStart commentRangeEnd commentReference docVars rsids documentProtection writeProtection attachedTemplate trackRevisions altChunk glossaryDocument instrText",&c.local))||
            (c.ns==S&&words("sheetCalcPr mruColors calcPr calcChain fileSharing customWorkbookViews pivotCaches pivotTableParts phoneticPr rPh sheetProtection workbookProtection",&c.local))||
            (c.ns==P&&words("custDataLst custData tags embeddedFontLst",&c.local));
        if drop {
            report.remove(
                part,
                "removed Office private/active carrier",
                String::from_utf8(serialize(&c)?)?,
            );
            continue;
        }
        ensure!(!(c.ns==W&&words("moveTo moveFromRangeStart moveFromRangeEnd moveToRangeStart moveToRangeEnd customXmlInsRangeStart customXmlDelRangeStart",&c.local)),"unsupported Word revision at {part}");
        if c.ns == W && words("ins fldSimple", &c.local) {
            ensure!(
                c.local != "fldSimple" || !c.string().trim().is_empty(),
                "Word field lacks displayed result at {part}"
            );
            report.remove(
                part,
                "flattened revision/field wrapper",
                format!("{:?}", c.attrs),
            );
            cleanse(&mut c, part, report)?;
            children.extend(c.children);
            continue;
        }
        if c.ns == S && c.local == "c" && c.children.iter().any(|x| x.ns == S && x.local == "f") {
            let value = c.children.iter().find(|x| x.ns == S && x.local == "v");
            ensure!(
                value.is_some_and(|v| !v.text.is_empty() || c.attr("t") == Some("str")),
                "uncached formula at {part}/cell/{}: recalculate and save locally, or exclude",
                c.attr("r").unwrap_or("?")
            );
            ensure!(
                words("n b e str s", c.attr("t").unwrap_or("n")),
                "unsupported formula cached type at {part}"
            );
            let old = std::mem::take(&mut c.children);
            for x in old {
                if x.ns == S && x.local == "f" {
                    report.remove(
                        &format!("{part}/cell/{}", c.attr("r").unwrap_or("?")),
                        "static snapshot: removed cell formula",
                        String::from_utf8(serialize(&x)?)?,
                    );
                } else {
                    c.children.push(x);
                }
            }
        }
        if (c.ns == S
            && words("definedName dataValidation cfRule", &c.local)
            && (formula(&c) || c.local == "definedName"))
            || (c.ns == X14 && words("dataValidation cfRule", &c.local) && formula(&c))
        {
            report.remove(
                part,
                "static snapshot: removed formula-bearing name/rule",
                String::from_utf8(serialize(&c)?)?,
            );
            continue;
        }
        if c.local == "extLst" {
            // Only explicitly implemented extension children may survive.
            let original = String::from_utf8(serialize(&c)?)?;
            for ext in &mut c.children {
                let ext_uri = ext.attr("uri").unwrap_or("").to_owned();
                ext.children.retain(|x| {
                    (x.ns == graphics::DSP && x.local == "dataModelExt" && ext_uri == graphics::DSP)
                        || (x.ns == P14 && x.local == "sectionLst")
                        || (x.ns == X14
                            && words("dataValidations conditionalFormattings", &x.local))
                });
            }
            c.children.retain(|x| !x.children.is_empty());
            if c.children.is_empty() {
                report.remove(part, "removed schema-ignorable extensions", original);
                continue;
            }
        }
        cleanse(&mut c, part, report)?;
        if c.ns == X14
            && c.local == "conditionalFormatting"
            && !c
                .children
                .iter()
                .any(|x| x.ns == X14 && x.local == "cfRule")
        {
            continue;
        }
        if (c.ns == S || c.ns == P) && words("ext extLst", &c.local) && c.children.is_empty() {
            report.remove(
                part,
                "removed empty Office extension wrapper",
                String::from_utf8(serialize(&c)?)?,
            );
            continue;
        }
        if (c.ns == S || c.ns == X14)
            && words(
                "dataValidations conditionalFormatting conditionalFormattings definedNames",
                &c.local,
            )
        {
            if c.children.is_empty() {
                continue;
            }
            if c.local == "dataValidations" {
                c.attrs.insert(
                    (String::new(), "count".into()),
                    c.children.len().to_string(),
                );
            }
        }
        children.push(c);
    }
    ensure!(field.is_none(), "unterminated Word field at {part}: safe displayed result requires begin/separate/end boundaries; recalculate and save locally, or exclude");
    n.children = children;
    let attrs = std::mem::take(&mut n.attrs);
    for ((ns, k), v) in attrs {
        if n.ignorable.contains(&ns)
            || (ns == W && (k.starts_with("rsid") || k == "displacedByCustomXml"))
            || (n.ns == S && ns.is_empty() && k == "codeName")
            || ns == "http://schemas.openxmlformats.org/markup-compatibility/2006"
            || (ns == "http://schemas.microsoft.com/office/word/2010/wordml"
                && words("paraId textId", &k))
        {
            report.remove(
                &format!("{part}/@{k}"),
                "removed private/compatibility attribute",
                v,
            );
        } else {
            n.attrs.insert((ns, k), v);
        }
    }
    Ok(())
}
fn validate_node(n: &Node, part: &str) -> Result<()> {
    if n.ns == A && n.local == "ext" && n.attr("uri").is_some() {
        ensure!(
            n.attr("uri") == Some(graphics::DSP)
                && n.children.len() == 1
                && n.children[0].ns == graphics::DSP
                && n.children[0].local == "dataModelExt",
            "unsupported SmartArt extension at {part}"
        );
    }
    if n.ns == A && n.local == "graphicData" {
        let uri = n
            .attr("uri")
            .context("drawing graphic data has no supported type URI")?;
        ensure!(
            n.children.len() == 1
                && ((uri == graphics::C
                    && n.children[0].ns == graphics::C
                    && n.children[0].local == "chart")
                    || (uri == graphics::DGM
                        && n.children[0].ns == graphics::DGM
                        && n.children[0].local == "relIds")
                    || (uri == PIC && n.children[0].ns == PIC && n.children[0].local == "pic")
                    || (uri == "http://schemas.openxmlformats.org/drawingml/2006/table"
                        && n.children[0].ns == A
                        && n.children[0].local == "tbl")),
            "unsupported drawing graphic data type at {part}"
        );
    }
    let xml_text = |s: &str| {
        s.chars().all(|c| {
            matches!(c, '\t' | '\r' | '\n')
                || (' '..='\u{d7ff}').contains(&c)
                || ('\u{e000}'..='\u{fffd}').contains(&c)
                || c >= '\u{10000}'
        })
    };
    ensure!(
        xml_text(&n.text) && n.attrs.values().all(|v| xml_text(v)),
        "invalid XML character at {part}"
    );
    if root_for(part) == Some((n.ns.as_str(), n.local.as_str())) {
        fn shapes(n: &Node, ids: &mut BTreeSet<String>) -> Result<()> {
            if [P, PIC, XDR].contains(&n.ns.as_str()) && n.local == "cNvPr" {
                let id = n.attr("id").context("shape lacks stable id")?;
                id.parse::<u32>().context("invalid shape id")?;
                ids.insert(id.to_owned());
            }
            for c in &n.children {
                shapes(c, ids)?;
            }
            Ok(())
        }
        shapes(n, &mut BTreeSet::new())
            .with_context(|| format!("ambiguous shape identity at {part}"))?;
    }
    ensure!(
        part_allows(part, &n.ns, &n.local),
        "element {{{}}}{} is not allowlisted for part {part}",
        n.ns,
        n.local
    );
    if words("t v", &n.local) && n.ns != graphics::DGM {
        ensure!(
            n.children.is_empty(),
            "mixed/structured text node refused at {part}"
        );
    }
    if n.ns == S && words("sheets tableColumns", &n.local) {
        let mut names = BTreeSet::new();
        ensure!(
            n.children.iter().all(|c| c
                .attr("name")
                .is_some_and(|name| names.insert(name.to_lowercase()))),
            "duplicate/absent sheet or column name at {part}"
        );
    }
    let allowed = rule(&n.ns, &n.local).with_context(|| {
        format!(
            "element {{{}}}{} has no Office allowlist at {part}",
            n.ns, n.local
        )
    })?;
    for (ns, k) in n.attrs.keys() {
        let layout = layout_allowlist::rule(&n.ns, &n.local).is_some_and(|attrs| words(attrs, k));
        if layout {
            ensure!(
                layout_allowlist::valid(&n.ns, &n.local, k, &n.attrs[&(ns.clone(), k.clone())]),
                "invalid layout attribute {k} on {} at {part}",
                n.local
            );
        }
        let permitted = if ns == XML {
            k == "space" && n.local == "t"
        } else if ns == R {
            words("id embed link dm lo qs cs",k)&&words("hyperlink headerReference footerReference sheet tablePart drawing sldId sldMasterId sldLayoutId notesMasterId handoutMasterId hlinkClick hlinkHover blip chart relIds", &n.local)
        } else {
            (if n.ns == W { ns == W } else { ns.is_empty() }) && (words(allowed, k) || layout)
        };
        ensure!(
            permitted,
            "attribute {{{ns}}}{k} on {} has no Office allowlist at {part}",
            n.local
        );
    }
    ensure!(
        n.text.trim().is_empty()
            || words(
                "t v oddHeader oddFooter evenHeader evenFooter firstHeader firstFooter align posOffset col row colOff rowOff sqref tableStyleId formatCode",
                &n.local
            ),
        "unexpected XML text at {part}/{}",
        n.local
    );
    if n.ns == S && n.local == "c" {
        let ty = n.attr("t").unwrap_or("n");
        ensure!(
            words("n b e str inlineStr", ty),
            "unsupported retained cell type at {part}"
        );
        if let Some(v) = n.children.iter().find(|c| c.ns == S && c.local == "v") {
            let valid = match ty {
                "n" => v.text.parse::<f64>().is_ok_and(f64::is_finite),
                "b" => words("0 1", &v.text),
                "e" => words(
                    "#NULL! #DIV/0! #VALUE! #REF! #NAME? #NUM! #N/A #GETTING_DATA",
                    &v.text,
                ),
                "str" => true,
                _ => false,
            };
            ensure!(
                valid,
                "invalid cached/static typed cell value at {part}/{}",
                n.attr("r").unwrap_or("?")
            );
        }
    }
    for c in &n.children {
        ensure!(
            child_allowed(n, c),
            "unsupported child {} in {} at {part}",
            c.local,
            n.local
        );
        validate_node(c, part)?;
    }
    Ok(())
}
fn child_allowed(parent: &Node, child: &Node) -> bool {
    let allowed=match(parent.ns.as_str(),parent.local.as_str()){
        (CT,"Types")=>Some((CT,"Default Override")),(REL,"Relationships")=>Some((REL,"Relationship")),
        (W,"document")=>Some((W,"body")),(W,"body")=>Some((W,"p tbl sectPr bookmarkStart bookmarkEnd")),
        (W,"p")=>Some((W,"pPr r hyperlink bookmarkStart bookmarkEnd")),
        (W,"r")=>Some((W,"rPr t tab br cr drawing lastRenderedPageBreak softHyphen noBreakHyphen footnoteReference endnoteReference separator continuationSeparator")),
        (W,"hyperlink")=>Some((W,"r bookmarkStart bookmarkEnd")),
        (S,"workbook")=>Some((S,"workbookPr bookViews sheets definedNames")),
        (S,"sheets")=>Some((S,"sheet")),(S,"sheetData")=>Some((S,"row")),(S,"row")=>Some((S,"c")),(S,"c")=>Some((S,"v is")),(S,"is"|"si")=>Some((S,"t r")),
        (P,"presentation")=>Some((P,"sldMasterIdLst notesMasterIdLst handoutMasterIdLst sldIdLst sldSz notesSz defaultTextStyle extLst")),
        (P,"sld"|"notes")=>Some((P,"cSld clrMapOvr extLst")),(P,"cSld")=>Some((P,"bg spTree extLst")),
        (P,"spTree")=>Some((P,"nvGrpSpPr grpSpPr sp pic grpSp graphicFrame")),
        (P,"sp")=>Some((P,"nvSpPr spPr style txBody extLst")),
        (A,"p")=>Some((A,"pPr r br endParaRPr")),(A,"r")=>Some((A,"rPr t")),
        _=>None,
    };
    if let Some((ns, names)) = allowed {
        child.ns == ns && words(names, &child.local)
    } else {
        true
    }
}
fn part_allows(part: &str, ns: &str, el: &str) -> bool {
    if part == "[Content_Types].xml" {
        return ns == CT;
    }
    if part.ends_with(".rels") {
        return ns == REL;
    }
    if part.contains("/theme/") {
        return ns == A;
    }
    if part == "word/settings.xml" {
        return ns==W&&(style_allowlist::settings_element(el)||words("settings doNotAutoCompressPictures zoom defaultTabStop characterSpacingControl evenAndOddHeaders displayBackgroundShape compat compatSetting proofState themeFontLang clrSchemeMapping decimalSymbol listSeparator view mirrorMargins autoHyphenation consecutiveHyphenLimit hyphenationZone doNotHyphenateCaps gutterAtTop defaultTableStyle doNotTrackMoves doNotTrackFormatting doNotUseHTMLParagraphAutoSpacing numFmt numStart numRestart pos footnote endnote",el));
    }
    if part == "word/fontTable.xml" {
        return ns == W
            && words(
                "fonts font altName panose1 charset family notTrueType pitch sig",
                el,
            );
    }
    if part == "ppt/tableStyles.xml" {
        return ns == A;
    }
    if ["word/styles.xml", "word/stylesWithEffects.xml"].contains(&part) {
        return ns == W
            && !words(
                "document body p r t tbl tr tc hdr ftr hyperlink drawing",
                el,
            );
    }
    if part == "word/numbering.xml" {
        return ns == W
            && !words(
                "document body p r t tbl tr tc hdr ftr hyperlink drawing",
                el,
            );
    }
    if part.contains("/diagrams/") {
        return [graphics::DGM, graphics::DSP, A].contains(&ns);
    }
    if part.contains("/charts/") {
        return [graphics::C, graphics::CS, A].contains(&ns);
    }
    if part.starts_with("xl/chartsheets/") {
        return ns == S;
    }
    if part.starts_with("word/") {
        return [W, A, WP, PIC, graphics::C, graphics::DGM].contains(&ns);
    }
    if part == "xl/workbook.xml" {
        return ns == S
            && words(
                "workbook workbookPr bookViews workbookView sheets sheet definedNames definedName",
                el,
            );
    }
    if part.starts_with("xl/worksheets/") {
        return (ns==S&&!words("workbook workbookPr bookViews workbookView sheets sheet sst si styleSheet fonts font fills fill borders border cellXfs cellStyleXfs cellStyles cellStyle numFmts numFmt table tableColumns tableColumn tableStyleInfo",el))||ns==X14||ns==XM;
    }
    if part == "xl/sharedStrings.xml" {
        return ns == S
            && words(
                "sst si r rPr t rFont b i u strike color sz family charset scheme",
                el,
            );
    }
    if part == "xl/styles.xml" {
        return ns==S&&!words("workbook worksheet sheet sheets row c v t is si sst sheetData table tableColumns tableColumn",el);
    }
    if part.starts_with("xl/tables/") {
        return ns==S&&words("table autoFilter filterColumn filters filter customFilters customFilter top10 dynamicFilter colorFilter iconFilter dateGroupItem sortState sortCondition tableColumns tableColumn tableStyleInfo extLst ext",el);
    }
    if part.starts_with("xl/drawings/") {
        return [XDR, A, PIC, graphics::C, graphics::DGM].contains(&ns);
    }
    if part == "ppt/presentation.xml" {
        return (ns==P&&words("presentation sldIdLst sldId sldSz notesSz sldMasterIdLst sldMasterId notesMasterIdLst notesMasterId handoutMasterIdLst handoutMasterId defaultTextStyle extLst ext",el))||ns==P14||ns==A;
    }
    if part.starts_with("ppt/") {
        return ns == P || ns == A || ns == graphics::C || ns == graphics::DGM;
    }
    false
}

fn content_attr(n: &Node, k: &str) -> bool {
    (n.ns == S && words("filter customFilter", &n.local) && k == "val") || (words(
        "displayName descr title tooltip display location errorTitle error promptTitle prompt text",
        k,
    )) || (k == "name"
        && ((n.ns == S && words("sheet table tableColumn", &n.local))
            || (n.ns == P && words("cSld cNvPr", &n.local))
            || (n.ns == P14 && n.local == "section")
            || ([WP, PIC, XDR, graphics::DSP].contains(&n.ns.as_str()) && words("docPr cNvPr", &n.local))))
        || (n.ns == REL && k == "Target" && n.attr("TargetMode") == Some("External"))
}
fn walk(
    mut n: Node,
    location: &str,
    report: &mut Report,
    edits: &BTreeMap<String, Edit>,
    applied: &mut BTreeSet<String>,
) -> Result<Node> {
    let mut result = Node {
        ns: n.ns.clone(),
        local: n.local.clone(),
        attrs: n.attrs.clone(),
        children: Vec::with_capacity(n.children.len()),
        text: n.text.clone(),
        ignorable: n.ignorable.clone(),
    };
    if n.ns == W && words("vanish webHidden", &n.local) {
        report.add(
            format!("{location}/@hidden"),
            "Office hidden status",
            Some(n.attr_ns(W, "val").unwrap_or("true").into()),
            false,
            true,
        )?;
    }
    for ((ns, k), value) in &mut result.attrs {
        let at = format!("{location}/@{{{ns}}}{k}");
        let content = content_attr(&n, k);
        if let Some(edit) = edits.get(&at) {
            ensure!(
                content
                    && words(
                    "descr title tooltip errorTitle error promptTitle prompt text name displayName",
                        k
                    ),
                "Office rename/reference edit needs an implemented dependency rewrite at {at}"
            );
            ensure!(
                crate::pipeline::sha256(value.as_bytes()) == edit.expected_sha256,
                "Office prior-value mismatch at {at}"
            );
            *value = edit.value.clone();
            applied.insert(at.clone());
        }
        report.add(
            at,
            "Office attribute",
            Some(value.clone()),
            content,
            content || words("hidden state show", k),
        )?;
    }
    if n.ns == S && n.local == "sheet" {
        let name = result.attr("name").context("sheet name absent")?;
        ensure!(
            !name.is_empty()
                && name.chars().count() <= 31
                && !name.contains(['[', ']', ':', '*', '?', '/', '\\']),
            "invalid sheet name at {location}"
        );
    }
    if n.ns == S && n.local == "table" {
        let name = result.attr("name").context("table name absent")?;
        ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
            "invalid table name at {location}"
        );
        ensure!(
            result.attr("displayName") == Some(name),
            "table name/displayName must be edited together at {location}"
        );
    }
    if n.ns == S && n.local == "tableColumn" {
        ensure!(
            result
                .attr("name")
                .is_some_and(|s| !s.is_empty() && s.chars().count() <= 255),
            "invalid column name at {location}"
        );
    }
    let heading = if n.ns == W && n.local == "p" {
        let style = n
            .children
            .iter()
            .find(|c| c.ns == W && c.local == "pPr")
            .and_then(|p| p.children.iter().find(|c| c.ns == W && c.local == "pStyle"))
            .and_then(|c| c.attr_ns(W, "val"));
        style
            .and_then(|s| {
                if s.eq_ignore_ascii_case("Title") {
                    Some(1)
                } else {
                    s.strip_prefix("Heading")
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|v| (1..=9).contains(v))
                }
            })
            .map(|level| format!("{} {}", "#".repeat(level), n.string()))
    } else if n.ns == S && n.local == "sheet" {
        n.attr("name").map(|s| format!("# {s}"))
    } else {
        None
    };
    let has_heading = heading.is_some();
    if let Some(heading) = heading {
        report.add(
            format!("{location}/@heading"),
            "Office title/heading",
            Some(heading),
            true,
            true,
        )?;
    }
    if !has_heading && ((n.ns == W || n.ns == A) && n.local == "p" || n.ns == S && n.local == "is")
    {
        let text = n.string();
        if !text.is_empty() {
            report.add(
                format!("{location}/@displayed-text"),
                "Office displayed text",
                Some(text),
                true,
                true,
            )?;
        }
    }
    if !n.text.is_empty() || words("t v", &n.local) {
        let at = format!("{location}/text");
        if let Some(edit) = edits.get(&at) {
            ensure!(
                n.local == "t"
                    || (n.ns == S && n.local == "v")
                    || (n.ns == graphics::C
                        && n.local == "v"
                        && (location.contains("}strLit[") || location.contains("}tx["))),
                "Office text node cannot be edited at {at}"
            );
            ensure!(
                crate::pipeline::sha256(n.text.as_bytes()) == edit.expected_sha256,
                "Office prior-value mismatch at {at}"
            );
            result.text = edit.value.clone();
            applied.insert(at.clone());
        }
        report.add(
            at,
            if n.ns == graphics::C
                && n.local == "v"
                && (location.contains("}strLit[") || location.contains("}tx["))
            {
                "Office text"
            } else if n.local == "v" {
                "Office typed value"
            } else {
                "Office text"
            },
            Some(result.text.clone()),
            words(
                "t v oddHeader oddFooter evenHeader evenFooter firstHeader firstFooter",
                &n.local,
            ) && ![
                format!("{{{W}}}p["),
                format!("{{{A}}}p["),
                format!("{{{S}}}is["),
            ]
            .iter()
            .any(|s| location.contains(s)),
            true,
        )?;
    }
    let mut counts = BTreeMap::<(String, String), usize>::new();
    let mut cell_refs = BTreeSet::new();
    for c in std::mem::take(&mut n.children) {
        let index = counts.entry((c.ns.clone(), c.local.clone())).or_default();
        *index += 1;
        let selector = if let Some(id) = c.shape_id() {
            format!("shape/{id}/{{{}}}{}[{}]", c.ns, c.local, index)
        } else if c.ns == S && c.local == "c" {
            let r = c.attr("r").context("cell lacks stable reference")?;
            ensure!(
                cell_refs.insert(r.to_owned()),
                "duplicate cell reference at {location}/{r}"
            );
            format!("cell/{r}")
        } else {
            format!("{{{}}}{}[{}]", c.ns, c.local, index)
        };
        result.children.push(walk(
            c,
            &format!("{location}/{selector}"),
            report,
            edits,
            applied,
        )?);
    }
    Ok(result)
}
fn read_package(input: &[u8]) -> Result<(BTreeMap<String, Vec<u8>>, u64)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(input))?;
    // ZipArchive indexes by name and collapses exact duplicates. Count the
    // original central-directory records before trusting that index.
    let mut central = usize::try_from(zip.central_directory_start())?;
    let mut entries = 0;
    while input.get(central..central + 4) == Some(b"PK\x01\x02") {
        ensure!(
            central + 46 <= input.len(),
            "truncated Office ZIP directory"
        );
        let size = |offset| {
            u16::from_le_bytes([input[central + offset], input[central + offset + 1]]) as usize
        };
        let length = 46 + size(28) + size(30) + size(32);
        central = central
            .checked_add(length)
            .context("Office ZIP directory overflow")?;
        ensure!(
            central <= input.len(),
            "truncated Office ZIP directory entry"
        );
        entries += 1;
        ensure!(
            entries <= MAX_PARTS,
            "OFFICE_PART_LIMIT_EXCEEDED: {entries} / {MAX_PARTS}"
        );
    }
    ensure!(entries == zip.len(), "duplicate Office ZIP member names");

    ensure!(
        zip.len() <= MAX_PARTS,
        "OFFICE_PART_LIMIT_EXCEEDED: {} / {MAX_PARTS}",
        zip.len()
    );
    let mut parts = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut part = zip.by_index(i)?;
        let name = part.name().to_owned();
        ensure!(
            seen.insert(path_key(name.trim_end_matches('/'))?),
            "case/canonical duplicate Office part {name}"
        );
        let mode = part.unix_mode().unwrap_or(0) & 0o170000;
        ensure!(
            mode == 0 || mode == 0o100000 || (mode == 0o040000 && part.is_dir()),
            "Office ZIP link/special entry at {name}"
        );
        ensure!(!part.encrypted(), "encrypted Office ZIP part {name}");
        if part.is_dir() {
            continue;
        }
        ensure!(
            part.size() <= MAX_BYTES,
            "OFFICE_DOCUMENT_BYTES_EXCEEDED at {name}: {} / {MAX_BYTES}",
            part.size()
        );
        ensure!(
            part.size() <= part.compressed_size().saturating_mul(100),
            "Office expansion ratio exceeds 100:1 at {name}"
        );
        let mut bytes = Vec::new();
        part.by_ref().take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == part.size(),
            "Office declared/actual size mismatch at {name}"
        );
        total = total
            .checked_add(bytes.len() as u64)
            .context("Office size overflow")?;
        ensure!(
            total <= MAX_BYTES,
            "OFFICE_DOCUMENT_BYTES_EXCEEDED at {name}: {total} / {MAX_BYTES}"
        );
        parts.insert(name, bytes);
    }
    Ok((parts, total))
}
fn media(input: &[u8], name: &str) -> Result<Vec<u8>> {
    Ok(filter_media(input, name)?.0)
}
fn filter_media(input: &[u8], name: &str) -> Result<(Vec<u8>, Vec<Removal>)> {
    let mut output = Vec::new();
    let mut removed = Vec::new();
    let mut record = |location: String, value: &[u8]| {
        removed.push(Removal {
            location,
            reason: "removed image metadata/trailing bytes".into(),
            value: String::from_utf8_lossy(value).into_owned(),
        })
    };
    let format = if name.ends_with(".png") {
        ensure!(
            input.starts_with(b"\x89PNG\r\n\x1a\n"),
            "invalid PNG signature at {name}"
        );
        output.extend_from_slice(&input[..8]);
        let mut pos = 8;
        let mut ended = false;
        while pos < input.len() {
            ensure!(pos + 12 <= input.len(), "truncated PNG at {name}");
            let len = u32::from_be_bytes(input[pos..pos + 4].try_into()?) as usize;
            let end = pos
                .checked_add(12)
                .and_then(|n| n.checked_add(len))
                .context("PNG chunk overflow")?;
            ensure!(end <= input.len(), "truncated PNG chunk at {name}");
            let kind = &input[pos + 4..pos + 8];
            ensure!(
                kind.iter().all(u8::is_ascii_alphabetic),
                "invalid PNG chunk type at {name}"
            );
            if [
                b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"sRGB", b"gAMA", b"cHRM", b"bKGD",
            ]
            .contains(&kind.try_into()?)
            {
                output.extend_from_slice(&input[pos..end]);
            } else {
                ensure!(
                    kind[0].is_ascii_lowercase(),
                    "unsupported required PNG chunk at {name}"
                );
                record(
                    format!("{name}/chunk/{}", String::from_utf8_lossy(kind)),
                    &input[pos + 8..end - 4],
                );
            }
            pos = end;
            if kind == b"IEND" {
                ended = true;
                break;
            }
        }
        ensure!(ended, "PNG lacks IEND at {name}");
        if pos < input.len() {
            record(format!("{name}/trailing"), &input[pos..]);
        }
        image::ImageFormat::Png
    } else if name.ends_with(".jpg") || name.ends_with(".jpeg") {
        ensure!(
            input.starts_with(&[0xff, 0xd8]),
            "invalid JPEG signature at {name}"
        );
        output.extend_from_slice(&input[..2]);
        let mut pos = 2;
        let mut ended = false;
        while pos < input.len() {
            let start = pos;
            ensure!(input[pos] == 0xff, "invalid JPEG marker at {name}");
            while pos < input.len() && input[pos] == 0xff {
                pos += 1;
            }
            let marker = *input.get(pos).context("truncated JPEG marker")?;
            pos += 1;
            if marker == 0xd9 {
                output.extend_from_slice(&[0xff, 0xd9]);
                ended = true;
                break;
            }
            ensure!(pos + 2 <= input.len(), "truncated JPEG segment at {name}");
            let len = u16::from_be_bytes(input[pos..pos + 2].try_into()?) as usize;
            let end = pos.checked_add(len).context("JPEG segment overflow")?;
            ensure!(
                len >= 2 && end <= input.len(),
                "invalid JPEG segment length at {name}"
            );
            let data = &input[pos + 2..end];
            match marker {
                0xe2 if data.starts_with(b"ICC_PROFILE") => bail!(
                    "unsupported JPEG ICC color profile at {name}; save as supported sRGB JPEG/PNG"
                ),
                0xe0 if data.starts_with(b"JFIF\0") => {
                    // Fixed JFIF decoding marker, with density/thumbnail metadata removed.
                    output.extend_from_slice(&[
                        255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
                    ]);
                    record(format!("{name}/APP0"), data);
                }
                0xee if data.starts_with(b"Adobe") => {
                    ensure!(
                        data.len() == 12,
                        "unsupported Adobe JPEG color marker at {name}"
                    );
                    ensure!(
                        data[11] <= 2,
                        "unsupported Adobe JPEG color transform at {name}"
                    );
                    output.extend_from_slice(&[
                        255, 238, 0, 14, 65, 100, 111, 98, 101, 0, 100, 0, 0, 0, 0, data[11],
                    ]);
                    record(format!("{name}/APP14/version-flags"), &data[..11]);
                }
                0xe0..=0xef | 0xfe => record(format!("{name}/marker/{marker:02x}"), data),
                0xc0..=0xc2 | 0xc4 | 0xda | 0xdb | 0xdd => {
                    output.extend_from_slice(&input[start..end])
                }
                _ => bail!("unsupported JPEG decoding marker {marker:02x} at {name}"),
            }
            pos = end;
            if marker == 0xda {
                let scan = pos;
                loop {
                    ensure!(pos + 1 < input.len(), "truncated JPEG scan at {name}");
                    if input[pos] != 0xff {
                        pos += 1;
                        continue;
                    }
                    let next = input[pos + 1];
                    if next == 0 || next == 0xff || (0xd0..=0xd7).contains(&next) {
                        pos += 2;
                        continue;
                    }
                    break;
                }
                output.extend_from_slice(&input[scan..pos]);
            }
        }
        ensure!(ended, "JPEG lacks EOI at {name}");
        if pos < input.len() {
            record(format!("{name}/trailing"), &input[pos..]);
        }
        image::ImageFormat::Jpeg
    } else {
        bail!("unsupported Office media format at {name}: only JPEG/PNG")
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(&output), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .with_context(|| format!("invalid Office image at {name}"))?;
    Ok((output, removed))
}

fn expected_type(path: &str, root: &Node, ext: &str) -> Result<&'static str> {
    Ok(match (root.ns.as_str(), root.local.as_str()) {
        (REL, "Relationships") => "application/vnd.openxmlformats-package.relationships+xml",
        (W, "document") if ext == "docx" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        }
        (W, "fonts") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"
        }
        (A, "tblStyleLst") => {
            "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"
        }
        (P, "presentationPr") => {
            "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml"
        }
        (W, "styles") if path == "word/stylesWithEffects.xml" => {
            "application/vnd.ms-word.stylesWithEffects+xml"
        }
        (W, "styles") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"
        }
        (W, "settings") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"
        }
        (W, "numbering") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"
        }
        (W, "hdr") => "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml",
        (W, "ftr") => "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml",
        (W, "footnotes") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"
        }
        (W, "endnotes") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"
        }
        (S, "workbook") if ext == "xlsx" => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
        }
        (S, "worksheet") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"
        }
        (S, "sst") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"
        }
        (S, "styleSheet") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"
        }
        (S, "table") => "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
        (P, "presentation") if ext == "pptx" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"
        }
        (P, "sld") => "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        (P, "sldMaster") => {
            "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"
        }
        (P, "sldLayout") => {
            "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"
        }
        (P, "notes") => {
            "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"
        }
        (P, "notesMaster") => {
            "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"
        }
        (P, "handoutMaster") => {
            "application/vnd.openxmlformats-officedocument.presentationml.handoutMaster+xml"
        }
        (graphics::DGM, "dataModel") => {
            "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"
        }
        (graphics::DGM, "layoutDef") => {
            "application/vnd.openxmlformats-officedocument.drawingml.diagramLayout+xml"
        }
        (graphics::DGM, "styleDef") => {
            "application/vnd.openxmlformats-officedocument.drawingml.diagramStyle+xml"
        }
        (graphics::DGM, "colorsDef") => {
            "application/vnd.openxmlformats-officedocument.drawingml.diagramColors+xml"
        }
        (graphics::DSP, "drawing") => "application/vnd.ms-office.drawingml.diagramDrawing+xml",
        (graphics::C, "chartSpace") => {
            "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"
        }
        (graphics::CS, "chartStyle") => "application/vnd.ms-office.chartstyle+xml",
        (graphics::CS, "colorStyle") => "application/vnd.ms-office.chartcolorstyle+xml",
        (S, "chartsheet") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"
        }
        (XDR, "wsDr") => "application/vnd.openxmlformats-officedocument.drawing+xml",
        (A, "theme") => "application/vnd.openxmlformats-officedocument.theme+xml",
        _ => bail!("unsupported/mismatched Office content type at {path}"),
    })
}
fn shared_strings(trees: &mut BTreeMap<String, Node>, report: &mut Report) -> Result<()> {
    let strings = trees
        .get("xl/sharedStrings.xml")
        .map(|n| n.children.clone())
        .unwrap_or_default();
    fn cells(n: &mut Node, strings: &[Node]) -> Result<()> {
        if n.ns == S && n.local == "c" && n.attr("t") == Some("s") {
            let v = n
                .children
                .iter()
                .find(|c| c.ns == S && c.local == "v")
                .context("shared-string cell lacks value")?;
            let index: usize = v.text.parse().context("invalid shared-string index")?;
            let si = strings
                .get(index)
                .context("dangling shared-string reference")?;
            ensure!(si.ns == S && si.local == "si", "invalid shared string item");
            let mut inline = si.clone();
            inline.local = "is".into();
            n.attrs
                .insert((String::new(), "t".into()), "inlineStr".into());
            n.children = vec![inline];
        }
        for c in &mut n.children {
            cells(c, strings)?;
        }
        Ok(())
    }
    for (part, n) in trees.iter_mut() {
        if part.starts_with("xl/worksheets/") {
            cells(n, &strings)?;
        }
    }
    if trees.remove("xl/sharedStrings.xml").is_some() {
        report.remove(
            "xl/sharedStrings.xml",
            "copied referenced shared strings into cells; removed orphan strings",
            String::from_utf8(serialize(&Node {
                ns: S.into(),
                local: "sst".into(),
                attrs: BTreeMap::new(),
                children: strings,
                text: String::new(),
                ignorable: Default::default(),
            })?)?,
        );
    }
    Ok(())
}
/// Rewrite, independently reopen, validate and inventory the delivered package.
/// The caller persists removals separately from the delivered package.
pub fn rebuild(
    name: &str,
    input: &[u8],
    edits: &BTreeMap<String, Edit>,
) -> Result<(Vec<u8>, Report)> {
    rebuild_inner(name,input,edits).map_err(|e|anyhow::anyhow!("This Office file cannot be safely prepared for listing: {name}: {e:#}. Save a supported, unencrypted .docx/.xlsx/.pptx without macros or external dependencies, then re-ingest, or exclude this file."))
}
pub fn inspect(name: &str, input: &[u8]) -> Result<Report> {
    let (parts, total) = read_package(input)?;
    validate_graph(&parts, name.rsplit('.').next().unwrap_or(""))?;
    let mut report = Report {
        allowlist_version: ALLOWLIST_VERSION,
        expanded_bytes: total,
        parts: parts.len(),
        source_parts: parts.len(),
        ..Report::default()
    };
    for (part, bytes) in &parts {
        report.add(
            part.clone(),
            "Office part",
            Some(part.clone()),
            false,
            false,
        )?;
        if part.contains("/media/") {
            media(bytes, part)?;
            continue;
        }
        ensure!(
            !removable(part),
            "removed Office carrier survived at {part}"
        );
        let root = parse(bytes, part)?;
        let (ns, local) = root_for(part).context("unsupported readback part")?;
        ensure!(
            root.ns == ns && root.local == local,
            "unsupported readback root at {part}"
        );
        validate_node(&root, part)?;
        ensure!(!formula(&root), "formula survived at {part}");
        let mut applied = BTreeSet::new();
        let root = walk(root, part, &mut report, &BTreeMap::new(), &mut applied)?;
        if part.ends_with(".rels") {
            let own = owner(part)?;
            for r in &root.children {
                if r.attr("TargetMode") != Some("External") {
                    let target =
                        resolve(&own, r.attr("Target").context("missing readback target")?)?;
                    ensure!(
                        parts.contains_key(&target),
                        "readback dangling target at {part}"
                    );
                }
            }
        }
    }
    for kind in [
        "author",
        "company",
        "template path",
        "source timestamps",
        "custom properties",
    ] {
        report.add(format!("@absent/{kind}"), kind, None, false, false)?;
    }
    ensure!(
        words("docx xlsx pptx", name.rsplit('.').next().unwrap_or("")),
        "unsupported Office extension"
    );
    Ok(report)
}
fn validate_graph(parts: &BTreeMap<String, Vec<u8>>, ext: &str) -> Result<()> {
    let ct = parse(
        parts
            .get("[Content_Types].xml")
            .context("missing readback content types")?,
        "[Content_Types].xml",
    )?;
    validate_node(&ct, "[Content_Types].xml")?;
    let mut declared = BTreeMap::new();
    for c in &ct.children {
        ensure!(
            c.ns == CT && c.local == "Override",
            "noncanonical rebuilt content types"
        );
        let p = c
            .attr("PartName")
            .context("missing part name")?
            .trim_start_matches('/');
        ensure!(
            parts.contains_key(p)
                && declared
                    .insert(
                        p.to_owned(),
                        c.attr("ContentType").context("missing type")?.to_owned()
                    )
                    .is_none(),
            "dangling/duplicate rebuilt content type"
        );
    }
    let mut trees = BTreeMap::new();
    let mut ids = BTreeMap::new();
    let mut root_target = None;
    let mut tables = BTreeSet::new();
    for (part, bytes) in parts {
        if part == "[Content_Types].xml" {
            continue;
        }
        let ty = declared.get(part).context("readback part lacks type")?;
        if part.contains("/media/") {
            ensure!(
                ty == if part.ends_with(".png") {
                    "image/png"
                } else {
                    "image/jpeg"
                },
                "mismatched rebuilt media type"
            );
            continue;
        }
        let n = parse(bytes, part)?;
        ensure!(
            ty == expected_type(part, &n, ext)?,
            "mismatched readback content type at {part}"
        );
        if n.ns == S && n.local == "table" {
            ensure!(
                n.attr("name")
                    .is_some_and(|s| tables.insert(s.to_lowercase())),
                "duplicate table name at {part}"
            );
        }
        if part.ends_with(".rels") {
            let own = owner(part)?;
            ensure!(
                own.is_empty() || parts.contains_key(&own),
                "orphan readback relationships at {part}"
            );
            let mut set = BTreeSet::new();
            for rel in &n.children {
                ensure!(
                    set.insert(
                        rel.attr("Id")
                            .context("missing readback relationship id")?
                            .to_owned()
                    ),
                    "duplicate readback relationship id"
                );
                let target = rel.attr("Target").context("missing readback target")?;
                if rel.attr("TargetMode") == Some("External") {
                    ensure!(
                        rel.attr("Type") == Some(&format!("{R}/hyperlink")) && web_target(target),
                        "unsupported external readback target"
                    );
                } else {
                    let target = resolve(&own, target)?;
                    ensure!(
                        parts.contains_key(&target),
                        "dangling readback relationship at {part}"
                    );
                    if part == "_rels/.rels"
                        && rel.attr("Type") == Some(&format!("{R}/officeDocument"))
                    {
                        ensure!(
                            root_target.replace(target).is_none(),
                            "multiple readback Office roots"
                        );
                    }
                }
            }
            ids.insert(own, set);
        }
        trees.insert(part.clone(), n);
    }
    let main = match ext {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        "pptx" => "ppt/presentation.xml",
        _ => bail!("unsupported readback extension"),
    };
    ensure!(
        root_target.as_deref() == Some(main),
        "missing/mismatched readback Office root"
    );
    graphics::validate_models(&trees)?;
    for (part, n) in &trees {
        validate_references(n, part, ids.get(part))?;
    }
    Ok(())
}
/// Value-only edits of a hash-verified, previously sanitized generation. Node
/// structure and binary media stay fixed. This grants no review authority.
pub fn edit(name: &str, input: &[u8], edits: &BTreeMap<String, Edit>) -> Result<(Vec<u8>, Report)> {
    inspect(name, input)?;
    let (parts, _) = read_package(input)?;
    let trees = parts
        .iter()
        .filter(|(p, _)| !p.contains("/media/"))
        .map(|(p, b)| Ok((p.clone(), parse(b, p)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let expanded_edits = graphics::paired_edits(&trees, edits)?;
    let edits = &expanded_edits;
    let mut applied = BTreeSet::new();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (part, bytes) in parts {
        let bytes = if part.contains("/media/") {
            bytes
        } else {
            let root = parse(&bytes, &part)?;
            let mut ignored = Report::default();
            let root = walk(root, &part, &mut ignored, edits, &mut applied)?;
            validate_node(&root, &part)?;
            serialize(&root)?
        };
        zip.start_file(
            part,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )?;
        zip.write_all(&bytes)?;
    }
    ensure!(applied.len() == edits.len(), "Office edit target missing");
    let output = zip.finish()?.into_inner();
    let report = inspect(name, &output)?;
    for (location, edit) in edits {
        ensure!(
            report
                .inventory
                .iter()
                .any(|v| &v.location == location && v.value.as_ref() == Some(&edit.value)),
            "Office edit readback mismatch at {location}"
        );
    }
    Ok((output, report))
}
fn rebuild_inner(
    name: &str,
    input: &[u8],
    edits: &BTreeMap<String, Edit>,
) -> Result<(Vec<u8>, Report)> {
    let ext = name.rsplit('.').next().unwrap_or("");
    ensure!(words("docx xlsx pptx", ext), "unsupported Office extension");
    let (mut parts, expanded) = read_package(input)?;
    let source_parts = parts.len();
    let mut report = Report::default();
    let mut trees = BTreeMap::new();
    let mut binaries = BTreeMap::new();
    graphics::prepare(&mut parts, &mut report)?;
    if edits
        .keys()
        .any(|key| key.starts_with("xl/") && key.contains("/cell/"))
    {
        if let Some(part) = parts
            .keys()
            .find(|p| p.contains("/charts/") && p.ends_with(".xml"))
        {
            bail!("XLSX cell edit cannot prove chart cache-to-source mapping at {part}/{{{}}}chart[1]; re-ingest the locally edited source or exclude",graphics::C);
        }
    }
    let ct = parse(
        parts
            .get("[Content_Types].xml")
            .context("missing content types")?,
        "[Content_Types].xml",
    )?;
    ensure!(
        ct.ns == CT && ct.local == "Types",
        "unsupported content types namespace"
    );
    validate_node(&ct, "[Content_Types].xml")?;
    let mut defaults = BTreeMap::new();
    let mut overrides = BTreeMap::new();
    for c in &ct.children {
        let ty = c
            .attr("ContentType")
            .context("content type absent")?
            .to_owned();
        ensure!(
            !ty.to_lowercase().contains("macro")
                && !ty.to_lowercase().contains("vba")
                && !ty.to_lowercase().contains("activex")
                && !ty.contains("oleObject"),
            "macro/active/embedded Office content type"
        );
        if c.local == "Default" {
            ensure!(
                defaults
                    .insert(
                        c.attr("Extension").context("missing extension")?.to_owned(),
                        ty
                    )
                    .is_none(),
                "duplicate content-type default"
            );
        } else if c.local == "Override" {
            let p = c
                .attr("PartName")
                .context("missing content type part")?
                .trim_start_matches('/');
            path_key(p)?;
            ensure!(parts.contains_key(p), "content type has dangling part {p}");
            ensure!(
                overrides.insert(p.to_owned(), ty).is_none(),
                "duplicate content-type override"
            );
        }
    }
    for (part, bytes) in &parts {
        let lower = part.to_lowercase();
        ensure!(
            !bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
                && !bytes.starts_with(b"PK\x03\x04"),
            "embedded/legacy binary object refused at {part}"
        );
        ensure!(
            !lower.contains("vbaproject")
                && !lower.contains("/activex/")
                && !lower.contains("/embeddings/")
                && !lower.starts_with("_xmlsignatures/"),
            "VBA/ActiveX/embedding/signature refused at {part}"
        );
        if part.starts_with("xl/drawings/") && part.ends_with(".vml") {
            let vml = parse(bytes, part)?;
            fn notes_only(n: &Node) -> bool {
                if words("OLEObject control", &n.local) {
                    return false;
                }
                if n.local == "shape"
                    && n.ns == "urn:schemas-microsoft-com:vml"
                    && !n.children.iter().any(|c| {
                        c.ns == "urn:schemas-microsoft-com:office:excel"
                            && c.local == "ClientData"
                            && c.attr("ObjectType") == Some("Note")
                    })
                {
                    return false;
                }
                n.children.iter().all(notes_only)
            }
            ensure!(notes_only(&vml), "non-comment VML dependency at {part}");
        }
        if removable(part) || part == "xl/calcChain.xml" {
            report.remove(
                part,
                "removed Office carrier",
                String::from_utf8_lossy(bytes).into_owned(),
            );
            continue;
        }
        if part == "[Content_Types].xml" {
            continue;
        }
        ensure!(
            part == "_rels/.rels"
                || part.starts_with(match ext {
                    "docx" => "word/",
                    "xlsx" => "xl/",
                    _ => "ppt/",
                }),
            "cross-format Office part {part}"
        );
        let ty = overrides
            .get(part)
            .or_else(|| part.rsplit('.').next().and_then(|ext| defaults.get(ext)))
            .context("part lacks content type")?;
        if part.contains("/media/") {
            let expected = if part.ends_with(".png") {
                "image/png"
            } else if part.ends_with(".jpg") || part.ends_with(".jpeg") {
                "image/jpeg"
            } else {
                bail!("unsupported Office media format at {part}: only JPEG/PNG; EMF/WMF/SVG are refused")
            };
            ensure!(ty == expected, "media content type mismatch at {part}");
            let (image, removals) = filter_media(bytes, part)?;
            report.removals.extend(removals);
            binaries.insert(part.clone(), image);
            report.warnings.push(format!(
                "{part}: visual review required, including cropped/hidden pixels"
            ));
            continue;
        }
        let (ns, local) =
            root_for(part).with_context(|| format!("unsupported Office part {part}"))?;
        let mut root = parse(bytes, part)?;
        ensure!(
            root.ns == ns && root.local == local,
            "unsupported/Strict/mismatched root at {part}"
        );
        ensure!(
            ty == expected_type(part, &root, ext)?,
            "mismatched content type at {part}"
        );
        if root.ns == S && root.local == "worksheet" {
            inspect_formulas(&root, part, &mut report)?;
        }
        cleanse(&mut root, part, &mut report)?;
        trees.insert(part.clone(), root);
    }
    if ext == "xlsx" {
        shared_strings(&mut trees, &mut report)?;
        report.warnings.push(
            "Excel values only: static cached snapshot; caches are not verified current".into(),
        );
    }
    // Neutral names for every non-fixed part. Root and standard fixed paths
    // remain stable; relative relationships are rewritten to absolute OPC paths.
    let mut rename = BTreeMap::new();
    let mut serial = 0;
    for part in trees.keys().chain(binaries.keys()) {
        if !part.ends_with(".rels")
            && (part.contains("/media/")
                || part.starts_with("xl/worksheets/")
                || part.starts_with("xl/tables/")
                || part.starts_with("xl/drawings/")
                || part.starts_with("word/header")
                || part.starts_with("word/footer")
                || part.starts_with("ppt/slides/")
                || part.starts_with("ppt/slideLayouts/")
                || part.starts_with("ppt/slideMasters/")
                || part.starts_with("ppt/notesSlides/")
                || part.starts_with("ppt/notesMasters/")
                || part.starts_with("ppt/handoutMasters/")
                || part.contains("/theme/")
                || part.contains("/charts/")
                || part.contains("/diagrams/")
                || part.starts_with("xl/chartsheets/"))
        {
            serial += 1;
            let (_, file) = part.rsplit_once('/').context("part path")?;
            let dir = if part.contains("/media/") {
                format!("{}/media", part.split('/').next().context("media root")?)
            } else if part.contains("/charts/")
                || part.contains("/diagrams/")
                || part.starts_with("xl/chartsheets/")
            {
                part.rsplit_once('/').unwrap().0.to_owned()
            } else if part.contains("/theme/") {
                format!("{}/theme", part.split('/').next().context("theme root")?)
            } else if part.starts_with("word/header") || part.starts_with("word/footer") {
                "word".into()
            } else {
                [
                    "xl/worksheets/",
                    "xl/tables/",
                    "xl/drawings/",
                    "ppt/slides/",
                    "ppt/slideLayouts/",
                    "ppt/slideMasters/",
                    "ppt/notesSlides/",
                    "ppt/notesMasters/",
                    "ppt/handoutMasters/",
                ]
                .into_iter()
                .find(|prefix| part.starts_with(prefix))
                .context("part has no neutral directory")?
                .trim_end_matches('/')
                .to_owned()
            };
            let suffix = file.rsplit('.').next().context("part suffix")?;
            let stem = if part.contains("/diagrams/") {
                file.trim_end_matches(".xml")
                    .trim_end_matches(|c: char| c.is_ascii_digit())
            } else if part.contains("/charts/") && file.starts_with("style") {
                "style"
            } else if part.contains("/charts/") && file.starts_with("colors") {
                "colors"
            } else if part.starts_with("word/header") {
                "header"
            } else if part.starts_with("word/footer") {
                "footer"
            } else {
                "part"
            };
            rename.insert(part.clone(), format!("{dir}/{stem}-{serial:06}.{suffix}"));
        }
    }
    for part in trees.keys().filter(|p| p.ends_with(".rels")) {
        let own = owner(part)?;
        if let Some(new) = rename.get(&own).cloned() {
            let (dir, file) = new.rsplit_once('/').context("renamed owner path")?;
            rename.insert(part.clone(), format!("{dir}/_rels/{file}.rels"));
        }
    }
    let mut removed_ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let present: BTreeSet<String> = trees.keys().chain(binaries.keys()).cloned().collect();
    let rel_parts: Vec<_> = trees
        .keys()
        .filter(|p| p.ends_with(".rels"))
        .cloned()
        .collect();
    let mut office_root = None;
    for part in rel_parts {
        let own = owner(&part)?;
        ensure!(
            own.is_empty() || present.contains(&own),
            "orphan relationship part {part}"
        );
        let root = trees.get_mut(&part).context("relationships absent")?;
        let mut ids = BTreeSet::new();
        let old = std::mem::take(&mut root.children);
        for mut rel in old {
            validate_node(&rel, &part)?;
            let id = rel
                .attr("Id")
                .context("missing relationship id")?
                .to_owned();
            ensure!(
                ids.insert(id.clone()),
                "duplicate relationship id at {part}/{id}"
            );
            let target = rel
                .attr("Target")
                .context("missing relationship target")?
                .to_owned();
            let ty = rel
                .attr("Type")
                .context("missing relationship type")?
                .to_owned();
            ensure!(
                (ty.starts_with(R) && ty.as_bytes().get(R.len()) == Some(&b'/'))
                    || ty == "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing"
                    || ["http://schemas.microsoft.com/office/2011/relationships/chartStyle", "http://schemas.microsoft.com/office/2011/relationships/chartColorStyle"].contains(&ty.as_str())
                    || (ty == "http://schemas.microsoft.com/office/2007/relationships/stylesWithEffects"
                        && resolve(&own, &target)? == "word/stylesWithEffects.xml"
                        && rel.attr("TargetMode") != Some("External"))
                    || (rel.attr("TargetMode") != Some("External")
                        && [
                            format!("{REL}/metadata/core-properties"),
                            format!("{REL}/metadata/thumbnail")
                        ]
                        .contains(&ty)
                        && removable(&resolve(&own, &target)?)),
                "unsupported relationship type at {part}/{id}"
            );
            let kind = ty.rsplit('/').next().unwrap_or("");
            let external = rel.attr("TargetMode") == Some("External");
            if external {
                if kind == "attachedTemplate" || (kind == "hyperlink" && !web_target(&target)) {
                    report.remove(
                        &format!("{part}/{id}"),
                        "removed non-web target/template",
                        target,
                    );
                    removed_ids.entry(own.clone()).or_default().insert(id);
                    continue;
                }
                ensure!(
                    kind == "hyperlink" && (web_target(&target)),
                    "external content dependency at {part}/{id}"
                );
            } else {
                ensure!(
                    rel.attr("TargetMode").is_none() || rel.attr("TargetMode") == Some("Internal"),
                    "unknown relationship mode at {part}"
                );
                let resolved = resolve(&own, &target)?;
                if removable(&resolved)
                    || resolved == "xl/calcChain.xml"
                    || resolved == "xl/sharedStrings.xml"
                {
                    report.remove(
                        &format!("{part}/{id}"),
                        "removed relationship to removed carrier",
                        target,
                    );
                    removed_ids.entry(own.clone()).or_default().insert(id);
                    continue;
                }
                ensure!(
                    present.contains(&resolved),
                    "dangling relationship at {part}/{id}: {resolved}"
                );
                ensure!(words("officeDocument styles stylesWithEffects settings numbering fontTable theme header footer footnotes endnotes worksheet sharedStrings table drawing slide slideMaster slideLayout notesSlide notesMaster handoutMaster presProps tableStyles image hyperlink chart chartsheet chartStyle chartColorStyle diagramData diagramLayout diagramQuickStyle diagramColors diagramDrawing",kind),"unsupported content dependency at {part}/{id}");
                if part == "_rels/.rels" && kind == "officeDocument" {
                    ensure!(
                        office_root.replace(resolved.clone()).is_none(),
                        "multiple Office roots"
                    );
                }
                rel.attrs.insert(
                    (String::new(), "Target".into()),
                    format!("/{}", rename.get(&resolved).unwrap_or(&resolved)),
                );
            }
            root.children.push(rel);
        }
    }
    let main = match ext {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        _ => "ppt/presentation.xml",
    };
    ensure!(
        office_root.as_deref() == Some(main) && trees.contains_key(main),
        "missing/mismatched Office root relationship"
    );
    fn unlink(n: &mut Node, ids: &BTreeSet<String>, part: &str) -> Result<()> {
        let mut out = vec![];
        for mut c in std::mem::take(&mut n.children) {
            let linked = c
                .attrs
                .iter()
                .any(|((ns, _), v)| ns == R && ids.contains(v));
            if linked {
                if c.ns == S && c.local == "pageSetup" {
                    c.attrs.retain(|(ns, _), v| ns != R || !ids.contains(v));
                    out.push(c);
                    continue;
                }
                if words("hyperlink hlinkClick hlinkHover", &c.local) {
                    if c.local == "hyperlink" {
                        out.extend(c.children);
                    }
                    continue;
                }
                bail!(
                    "dependent content cannot survive removed relationship at {part}/{}",
                    c.local
                );
            }
            unlink(&mut c, ids, part)?;
            out.push(c);
        }
        n.children = out;
        Ok(())
    }
    let relationship_ids: BTreeMap<String, BTreeSet<String>> = trees
        .iter()
        .filter(|(p, _)| p.ends_with(".rels"))
        .map(|(p, n)| {
            Ok((
                owner(p)?,
                n.children
                    .iter()
                    .map(|r| r.attr("Id").context("missing id").map(str::to_owned))
                    .collect::<Result<_>>()?,
            ))
        })
        .collect::<Result<_>>()?;
    let mut rewritten = BTreeMap::new();
    let mut applied = BTreeSet::new();
    for (part, mut root) in trees {
        if let Some(ids) = removed_ids.get(&part) {
            unlink(&mut root, ids, &part)?;
        }
        validate_node(&root, &part)?;
        validate_references(&root, &part, relationship_ids.get(&part))?;
        let delivered = rename.get(&part).unwrap_or(&part);
        // Values are read back again below. This first walk applies bound edits.
        let mut ignored = Report::default();
        let edited = walk(root, delivered, &mut ignored, edits, &mut applied)?;
        rewritten.insert(delivered.clone(), serialize(&edited)?);
    }
    ensure!(
        applied.len() == edits.len(),
        "Office edit selector missing or ambiguous"
    );
    for (part, bytes) in binaries {
        rewritten.insert(rename.get(&part).unwrap_or(&part).clone(), bytes);
    }
    // Rebuild the type table from retained types; no source type identifiers.
    let mut new_ct = Node {
        ns: CT.into(),
        local: "Types".into(),
        attrs: BTreeMap::new(),
        children: vec![],
        text: String::new(),
        ignorable: Default::default(),
    };
    for (part, bytes) in &rewritten {
        let ty = if part.ends_with(".png") {
            "image/png"
        } else if part.ends_with(".jpg") || part.ends_with(".jpeg") {
            "image/jpeg"
        } else {
            expected_type(part, &parse(bytes, part)?, ext)?
        };
        new_ct.children.push(Node {
            ns: CT.into(),
            local: "Override".into(),
            attrs: BTreeMap::from([
                ((String::new(), "PartName".into()), format!("/{part}")),
                ((String::new(), "ContentType".into()), ty.into()),
            ]),
            children: vec![],
            text: String::new(),
            ignorable: Default::default(),
        });
    }
    rewritten.insert("[Content_Types].xml".into(), serialize(&new_ct)?);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (part, bytes) in &rewritten {
        zip.start_file(
            part,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )?;
        zip.write_all(bytes)?;
    }
    let output = zip.finish()?.into_inner();
    // The independent walk reopens ZIP headers, type table and relationships.
    let reopened = inspect(name, &output)?;
    report.allowlist_version = ALLOWLIST_VERSION;
    report.inventory = reopened.inventory;
    report.expanded_bytes = reopened.expanded_bytes.max(expanded);
    report.parts = reopened.parts;
    report.source_parts = source_parts;
    for (location, edit) in edits {
        ensure!(
            report
                .inventory
                .iter()
                .any(|v| &v.location == location && v.value.as_ref() == Some(&edit.value)),
            "Office edit failed readback at {location}"
        );
    }
    Ok((output, report))
}

fn validate_references(root: &Node, part: &str, ids: Option<&BTreeSet<String>>) -> Result<()> {
    for ((ns, _), id) in &root.attrs {
        if ns == R {
            ensure!(
                ids.is_some_and(|set| set.contains(id)),
                "dangling relationship id at {part}: {id}"
            );
        }
    }
    if root.ns == W && root.local == "hyperlink" {
        ensure!(
            root.attr_ns(W, "anchor").is_none(),
            "internal Word hyperlink requires supported bookmark validation at {part}"
        );
    }
    if root.ns == S && root.local == "hyperlink" {
        ensure!(
            root.attr("location").is_none(),
            "internal worksheet hyperlink requires supported target validation at {part}"
        );
    }
    for c in &root.children {
        validate_references(c, part, ids)?;
    }
    Ok(())
}

#[cfg(test)]
mod media_tests {
    use super::*;
    #[test]
    fn png_icc_profile_is_removed_and_pixels_survive() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            8,
            image::Rgb([30, 90, 150]),
        ));
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let encoded = encoded.into_inner();
        let marker = b"planted-ICC-profile-1927";
        let mut input = encoded[..33].to_vec(); // signature + IHDR
        input.extend_from_slice(&(marker.len() as u32).to_be_bytes());
        input.extend_from_slice(b"iCCP");
        input.extend_from_slice(marker);
        // The whole ancillary chunk is discarded before image decoding.
        input.extend_from_slice(&[0; 4]);
        input.extend_from_slice(&encoded[33..]);
        let (output, removals) = filter_media(&input, "image.png").unwrap();
        assert!(removals
            .iter()
            .any(|r| r.value.contains("planted-ICC-profile-1927")));
        assert_eq!(
            image::load_from_memory(&output).unwrap().to_rgb8(),
            image.to_rgb8()
        );
        assert!(!output.windows(marker.len()).any(|w| w == marker));
    }
    #[test]
    fn jpeg_metadata_filter_preserves_decoded_pixels() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            8,
            image::Rgb([30, 90, 150]),
        ));
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Jpeg)
            .unwrap();
        let encoded = encoded.into_inner();
        let marker = b"planted-jpeg-user-S1927";
        let mut input = encoded[..2].to_vec();
        input.extend_from_slice(&[255, 225]);
        input.extend_from_slice(&((marker.len() + 2) as u16).to_be_bytes());
        input.extend_from_slice(marker);
        input.extend_from_slice(&encoded[2..]);
        input.extend_from_slice(marker);
        let (clean, removed) = filter_media(&input, "word/media/private.jpg").unwrap();
        assert!(!clean.windows(marker.len()).any(|v| v == marker));
        assert!(removed
            .iter()
            .any(|v| v.value.contains("planted-jpeg-user-S1927")));
        assert_eq!(
            image::load_from_memory(&encoded).unwrap().to_rgb8(),
            image::load_from_memory(&clean).unwrap().to_rgb8()
        );
        let mut bad = encoded[..2].to_vec();
        bad.extend_from_slice(&[255, 226, 0, 13]);
        bad.extend_from_slice(b"ICC_PROFILE");
        bad.extend_from_slice(&encoded[2..]);
        assert!(filter_media(&bad, "word/media/private.jpg")
            .err()
            .unwrap()
            .to_string()
            .contains("ICC color profile"));
    }
}
