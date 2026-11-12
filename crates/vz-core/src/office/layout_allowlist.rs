//! Generated selected layout subset; see scripts/office-layout-allowlist.py.
//! Open XML SDK 431ab05cf160248cc3885a4a766026d4f8243792 (MIT; OPEN_XML_SDK_LICENSE.txt).
//! No string, identifier, relationship, target or text-bearing attributes.
use super::{A, P, PIC, S, W, XDR};
pub(super) fn rule(ns: &str, element: &str) -> Option<&'static str> {
    Some(match (ns, element) {
        (A, "alphaBiLevel") => "thresh",
        (A, "alphaCeiling") => "",
        (A, "alphaFloor") => "",
        (A, "alphaInv") => "",
        (A, "alphaModFix") => "amt",
        (A, "alphaOff") => "val",
        (A, "alphaOutset") => "rad",
        (A, "alphaRepl") => "a",
        (A, "biLevel") => "thresh",
        (A, "blend") => "blend",
        (A, "blipFill") => "dpi rotWithShape",
        (A, "blur") => "rad grow",
        (A, "bottom") => "",
        (A, "buAutoNum") => "type startAt",
        (A, "buClrTx") => "",
        (A, "buFontTx") => "",
        (A, "buNone") => "",
        (A, "buSzPct") => "val",
        (A, "buSzPts") => "val",
        (A, "buSzTx") => "",
        (A, "cNvPicPr") => "preferRelativeResize",
        (A, "cell3D") => "prstMaterial",
        (A, "clrChange") => "useA",
        (A, "clrRepl") => "",
        (A, "comp") => "",
        (A, "custDash") => "",
        (A, "cxnSpLocks") => "noGrp noSelect noRot noChangeAspect noMove noResize noEditPoints noAdjustHandles noChangeArrowheads noChangeShapeType",
        (A, "duotone") => "",
        (A, "fill") => "",
        (A, "fillOverlay") => "blend",
        (A, "gamma") => "",
        (A, "glow") => "rad",
        (A, "gradFill") => "flip rotWithShape",
        (A, "graphicFrameLocks") => "noGrp noDrilldown noSelect noChangeAspect noMove noResize",
        (A, "gray") => "",
        (A, "grayscl") => "",
        (A, "gridCol") => "w",
        (A, "grpFill") => "",
        (A, "grpSpLocks") => "noGrp noUngrp noSelect noRot noChangeAspect noMove noResize",
        (A, "highlight") => "",
        (A, "hsl") => "hue sat lum",
        (A, "hslClr") => "hue sat lum",
        (A, "hue") => "val",
        (A, "hueOff") => "val",
        (A, "innerShdw") => "blurRad dist dir",
        (A, "inv") => "",
        (A, "invGamma") => "",
        (A, "left") => "",
        (A, "lin") => "ang scaled",
        (A, "lnB") => "w cap cmpd algn",
        (A, "lnBlToTr") => "w cap cmpd algn",
        (A, "lnL") => "w cap cmpd algn",
        (A, "lnR") => "w cap cmpd algn",
        (A, "lnT") => "w cap cmpd algn",
        (A, "lnTlToBr") => "w cap cmpd algn",
        (A, "miter") => "lim",
        (A, "noAutofit") => "",
        (A, "normAutofit") => "fontScale lnSpcReduction",
        (A, "outerShdw") => "blurRad dist dir sx sy kx ky algn rotWithShape",
        (A, "pattFill") => "prst",
        (A, "picLocks") => "noGrp noSelect noRot noChangeAspect noMove noResize noEditPoints noAdjustHandles noChangeArrowheads noChangeShapeType noCrop",
        (A, "prstClr") => "val",
        (A, "prstDash") => "val",
        (A, "prstGeom") => "prst",
        (A, "prstShdw") => "prst dist dir",
        (A, "prstTxWarp") => "prst",
        (A, "reflection") => "blurRad stA stPos endA endPos dist dir fadeDir sx sy kx ky algn rotWithShape",
        (A, "relOff") => "tx ty",
        (A, "right") => "",
        (A, "round") => "",
        (A, "schemeClr") => "val",
        (A, "scrgbClr") => "r g b",
        (A, "softEdge") => "rad",
        (A, "spAutoFit") => "",
        (A, "spLocks") => "noGrp noSelect noRot noChangeAspect noMove noResize noEditPoints noAdjustHandles noChangeArrowheads noChangeShapeType noTextEdit",
        (A, "spPr") => "bwMode",
        (A, "spcPct") => "val",
        (A, "spcPts") => "val",
        (A, "tab") => "pos algn",
        (A, "tabLst") => "",
        (A, "tbl") => "",
        (A, "tblGrid") => "",
        (A, "tc") => "rowSpan gridSpan hMerge vMerge",
        (A, "tcBdr") => "",
        (A, "tcPr") => "marL marR marT marB vert anchor anchorCtr horzOverflow",
        (A, "tile") => "tx ty sx sy flip algn",
        (A, "top") => "",
        (A, "tr") => "h",
        (A, "uFill") => "",
        (A, "uFillTx") => "",
        (A, "uLnTx") => "",
        (P, "blipFill") => "dpi rotWithShape",
        (P, "cNvPicPr") => "preferRelativeResize",
        (P, "notes") => "showMasterSp showMasterPhAnim",
        (P, "penClr") => "",
        (P, "spPr") => "bwMode",
        (PIC, "blipFill") => "dpi rotWithShape",
        (PIC, "cNvPicPr") => "preferRelativeResize",
        (PIC, "spPr") => "bwMode",
        (S, "horizontal") => "style",
        (S, "vertical") => "style",
        (S, "workbookPr") => "date1904 dateCompatibility showObjects showBorderUnselectedTables filterPrivacy showInkAnnotation hidePivotFieldList showPivotChartFilter autoCompressPictures defaultThemeVersion",
        (W, "bdo") => "val",
        (W, "dir") => "val",
        (W, "hMerge") => "val",
        (W, "hideMark") => "val",
        (W, "noEndnote") => "val",
        (W, "noWrap") => "val",
        (W, "pgBorders") => "zOrder display offsetFrom",
        (W, "ptab") => "alignment relativeTo leader",
        (W, "tblCellMar") => "",
        (W, "tblInd") => "w type",
        (W, "tblLayout") => "type",
        (W, "tblOverlap") => "val",
        (W, "tblpPr") => "leftFromText rightFromText topFromText bottomFromText vertAnchor horzAnchor tblpXSpec tblpX tblpYSpec tblpY",
        (W, "tcMar") => "",
        (W, "textAlignment") => "val",
        (W, "textboxTightWrap") => "val",
        (W, "trHeight") => "val hRule",
        (W, "vMerge") => "val",
        (XDR, "blipFill") => "rotWithShape",
        (XDR, "cNvPicPr") => "preferRelativeResize",
        (XDR, "spPr") => "bwMode",
        _ => return None,
    })
}
pub(super) fn valid(ns: &str, element: &str, attr: &str, value: &str) -> bool {
    match (ns, element, attr) {
        (A, "alphaBiLevel", "thresh") => value.parse::<i32>().is_ok(),
        (A, "alphaModFix", "amt") => value.parse::<i32>().is_ok(),
        (A, "alphaOff", "val") => value.parse::<i32>().is_ok(),
        (A, "alphaOutset", "rad") => value.parse::<i64>().is_ok(),
        (A, "alphaRepl", "a") => value.parse::<i32>().is_ok(),
        (A, "biLevel", "thresh") => value.parse::<i32>().is_ok(),
        (A, "blend", "blend") => matches!(value, "over" | "mult" | "screen" | "darken" | "lighten"),
        (A, "blipFill", "dpi") => value.parse::<u32>().is_ok(),
        (A, "blipFill", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "blur", "rad") => value.parse::<i64>().is_ok(),
        (A, "blur", "grow") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "buAutoNum", "type") => matches!(
            value,
            "alphaLcParenBoth"
                | "alphaUcParenBoth"
                | "alphaLcParenR"
                | "alphaUcParenR"
                | "alphaLcPeriod"
                | "alphaUcPeriod"
                | "arabicParenBoth"
                | "arabicParenR"
                | "arabicPeriod"
                | "arabicPlain"
                | "romanLcParenBoth"
                | "romanUcParenBoth"
                | "romanLcParenR"
                | "romanUcParenR"
                | "romanLcPeriod"
                | "romanUcPeriod"
                | "circleNumDbPlain"
                | "circleNumWdBlackPlain"
                | "circleNumWdWhitePlain"
                | "arabicDbPeriod"
                | "arabicDbPlain"
                | "ea1ChsPeriod"
                | "ea1ChsPlain"
                | "ea1ChtPeriod"
                | "ea1ChtPlain"
                | "ea1JpnChsDbPeriod"
                | "ea1JpnKorPlain"
                | "ea1JpnKorPeriod"
                | "arabic1Minus"
                | "arabic2Minus"
                | "hebrew2Minus"
                | "thaiAlphaPeriod"
                | "thaiAlphaParenR"
                | "thaiAlphaParenBoth"
                | "thaiNumPeriod"
                | "thaiNumParenR"
                | "thaiNumParenBoth"
                | "hindiAlphaPeriod"
                | "hindiNumPeriod"
                | "hindiNumParenR"
                | "hindiAlpha1Period"
        ),
        (A, "buAutoNum", "startAt") => value.parse::<i32>().is_ok(),
        (A, "buSzPct", "val") => value.parse::<i32>().is_ok(),
        (A, "buSzPts", "val") => value.parse::<i32>().is_ok(),
        (A, "cNvPicPr", "preferRelativeResize") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cell3D", "prstMaterial") => matches!(
            value,
            "legacyMatte"
                | "legacyPlastic"
                | "legacyMetal"
                | "legacyWireframe"
                | "matte"
                | "plastic"
                | "metal"
                | "warmMatte"
                | "translucentPowder"
                | "powder"
                | "dkEdge"
                | "softEdge"
                | "clear"
                | "flat"
                | "softmetal"
        ),
        (A, "clrChange", "useA") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noGrp") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noSelect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noRot") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noChangeAspect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noMove") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noResize") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noEditPoints") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noAdjustHandles") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noChangeArrowheads") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "cxnSpLocks", "noChangeShapeType") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "fillOverlay", "blend") => {
            matches!(value, "over" | "mult" | "screen" | "darken" | "lighten")
        }
        (A, "glow", "rad") => value.parse::<i64>().is_ok(),
        (A, "gradFill", "flip") => matches!(value, "none" | "x" | "y" | "xy"),
        (A, "gradFill", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "graphicFrameLocks", "noGrp") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "graphicFrameLocks", "noDrilldown") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "graphicFrameLocks", "noSelect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "graphicFrameLocks", "noChangeAspect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "graphicFrameLocks", "noMove") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "graphicFrameLocks", "noResize") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "gridCol", "w") => value.parse::<i64>().is_ok(),
        (A, "grpSpLocks", "noGrp") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "grpSpLocks", "noUngrp") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "grpSpLocks", "noSelect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "grpSpLocks", "noRot") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "grpSpLocks", "noChangeAspect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "grpSpLocks", "noMove") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "grpSpLocks", "noResize") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "hsl", "hue") => value.parse::<i32>().is_ok(),
        (A, "hsl", "sat") => value.parse::<i32>().is_ok(),
        (A, "hsl", "lum") => value.parse::<i32>().is_ok(),
        (A, "hslClr", "hue") => value.parse::<i32>().is_ok(),
        (A, "hslClr", "sat") => value.parse::<i32>().is_ok(),
        (A, "hslClr", "lum") => value.parse::<i32>().is_ok(),
        (A, "hue", "val") => value.parse::<i32>().is_ok(),
        (A, "hueOff", "val") => value.parse::<i32>().is_ok(),
        (A, "innerShdw", "blurRad") => value.parse::<i64>().is_ok(),
        (A, "innerShdw", "dist") => value.parse::<i64>().is_ok(),
        (A, "innerShdw", "dir") => value.parse::<i32>().is_ok(),
        (A, "lin", "ang") => value.parse::<i32>().is_ok(),
        (A, "lin", "scaled") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "lnB", "w") => value.parse::<i32>().is_ok(),
        (A, "lnB", "cap") => matches!(value, "rnd" | "sq" | "flat"),
        (A, "lnB", "cmpd") => matches!(value, "sng" | "dbl" | "thickThin" | "thinThick" | "tri"),
        (A, "lnB", "algn") => matches!(value, "ctr" | "in"),
        (A, "lnBlToTr", "w") => value.parse::<i32>().is_ok(),
        (A, "lnBlToTr", "cap") => matches!(value, "rnd" | "sq" | "flat"),
        (A, "lnBlToTr", "cmpd") => {
            matches!(value, "sng" | "dbl" | "thickThin" | "thinThick" | "tri")
        }
        (A, "lnBlToTr", "algn") => matches!(value, "ctr" | "in"),
        (A, "lnL", "w") => value.parse::<i32>().is_ok(),
        (A, "lnL", "cap") => matches!(value, "rnd" | "sq" | "flat"),
        (A, "lnL", "cmpd") => matches!(value, "sng" | "dbl" | "thickThin" | "thinThick" | "tri"),
        (A, "lnL", "algn") => matches!(value, "ctr" | "in"),
        (A, "lnR", "w") => value.parse::<i32>().is_ok(),
        (A, "lnR", "cap") => matches!(value, "rnd" | "sq" | "flat"),
        (A, "lnR", "cmpd") => matches!(value, "sng" | "dbl" | "thickThin" | "thinThick" | "tri"),
        (A, "lnR", "algn") => matches!(value, "ctr" | "in"),
        (A, "lnT", "w") => value.parse::<i32>().is_ok(),
        (A, "lnT", "cap") => matches!(value, "rnd" | "sq" | "flat"),
        (A, "lnT", "cmpd") => matches!(value, "sng" | "dbl" | "thickThin" | "thinThick" | "tri"),
        (A, "lnT", "algn") => matches!(value, "ctr" | "in"),
        (A, "lnTlToBr", "w") => value.parse::<i32>().is_ok(),
        (A, "lnTlToBr", "cap") => matches!(value, "rnd" | "sq" | "flat"),
        (A, "lnTlToBr", "cmpd") => {
            matches!(value, "sng" | "dbl" | "thickThin" | "thinThick" | "tri")
        }
        (A, "lnTlToBr", "algn") => matches!(value, "ctr" | "in"),
        (A, "miter", "lim") => value.parse::<i32>().is_ok(),
        (A, "normAutofit", "fontScale") => value.parse::<i32>().is_ok(),
        (A, "normAutofit", "lnSpcReduction") => value.parse::<i32>().is_ok(),
        (A, "outerShdw", "blurRad") => value.parse::<i64>().is_ok(),
        (A, "outerShdw", "dist") => value.parse::<i64>().is_ok(),
        (A, "outerShdw", "dir") => value.parse::<i32>().is_ok(),
        (A, "outerShdw", "sx") => value.parse::<i32>().is_ok(),
        (A, "outerShdw", "sy") => value.parse::<i32>().is_ok(),
        (A, "outerShdw", "kx") => value.parse::<i32>().is_ok(),
        (A, "outerShdw", "ky") => value.parse::<i32>().is_ok(),
        (A, "outerShdw", "algn") => matches!(
            value,
            "tl" | "t" | "tr" | "l" | "ctr" | "r" | "bl" | "b" | "br"
        ),
        (A, "outerShdw", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "pattFill", "prst") => matches!(
            value,
            "pct5"
                | "pct10"
                | "pct20"
                | "pct25"
                | "pct30"
                | "pct40"
                | "pct50"
                | "pct60"
                | "pct70"
                | "pct75"
                | "pct80"
                | "pct90"
                | "horz"
                | "vert"
                | "ltHorz"
                | "ltVert"
                | "dkHorz"
                | "dkVert"
                | "narHorz"
                | "narVert"
                | "dashHorz"
                | "dashVert"
                | "cross"
                | "dnDiag"
                | "upDiag"
                | "ltDnDiag"
                | "ltUpDiag"
                | "dkDnDiag"
                | "dkUpDiag"
                | "wdDnDiag"
                | "wdUpDiag"
                | "dashDnDiag"
                | "dashUpDiag"
                | "diagCross"
                | "smCheck"
                | "lgCheck"
                | "smGrid"
                | "lgGrid"
                | "dotGrid"
                | "smConfetti"
                | "lgConfetti"
                | "horzBrick"
                | "diagBrick"
                | "solidDmnd"
                | "openDmnd"
                | "dotDmnd"
                | "plaid"
                | "sphere"
                | "weave"
                | "divot"
                | "shingle"
                | "wave"
                | "trellis"
                | "zigZag"
        ),
        (A, "picLocks", "noGrp") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noSelect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noRot") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noChangeAspect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noMove") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noResize") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noEditPoints") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noAdjustHandles") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noChangeArrowheads") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noChangeShapeType") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "picLocks", "noCrop") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "prstClr", "val") => matches!(
            value,
            "aliceBlue"
                | "antiqueWhite"
                | "aqua"
                | "aquamarine"
                | "azure"
                | "beige"
                | "bisque"
                | "black"
                | "blanchedAlmond"
                | "blue"
                | "blueViolet"
                | "brown"
                | "burlyWood"
                | "cadetBlue"
                | "chartreuse"
                | "chocolate"
                | "coral"
                | "cornflowerBlue"
                | "cornsilk"
                | "crimson"
                | "cyan"
                | "dkBlue"
                | "dkCyan"
                | "dkGoldenrod"
                | "dkGray"
                | "dkGreen"
                | "dkKhaki"
                | "dkMagenta"
                | "dkOliveGreen"
                | "dkOrange"
                | "dkOrchid"
                | "dkRed"
                | "dkSalmon"
                | "dkSeaGreen"
                | "dkSlateBlue"
                | "dkSlateGray"
                | "dkTurquoise"
                | "dkViolet"
                | "deepPink"
                | "deepSkyBlue"
                | "dimGray"
                | "dodgerBlue"
                | "firebrick"
                | "floralWhite"
                | "forestGreen"
                | "fuchsia"
                | "gainsboro"
                | "ghostWhite"
                | "gold"
                | "goldenrod"
                | "gray"
                | "green"
                | "greenYellow"
                | "honeydew"
                | "hotPink"
                | "indianRed"
                | "indigo"
                | "ivory"
                | "khaki"
                | "lavender"
                | "lavenderBlush"
                | "lawnGreen"
                | "lemonChiffon"
                | "ltBlue"
                | "ltCoral"
                | "ltCyan"
                | "ltGoldenrodYellow"
                | "ltGray"
                | "ltGreen"
                | "ltPink"
                | "ltSalmon"
                | "ltSeaGreen"
                | "ltSkyBlue"
                | "ltSlateGray"
                | "ltSteelBlue"
                | "ltYellow"
                | "lime"
                | "limeGreen"
                | "linen"
                | "magenta"
                | "maroon"
                | "medAquamarine"
                | "medBlue"
                | "medOrchid"
                | "medPurple"
                | "medSeaGreen"
                | "medSlateBlue"
                | "medSpringGreen"
                | "medTurquoise"
                | "medVioletRed"
                | "midnightBlue"
                | "mintCream"
                | "mistyRose"
                | "moccasin"
                | "navajoWhite"
                | "navy"
                | "oldLace"
                | "olive"
                | "oliveDrab"
                | "orange"
                | "orangeRed"
                | "orchid"
                | "paleGoldenrod"
                | "paleGreen"
                | "paleTurquoise"
                | "paleVioletRed"
                | "papayaWhip"
                | "peachPuff"
                | "peru"
                | "pink"
                | "plum"
                | "powderBlue"
                | "purple"
                | "red"
                | "rosyBrown"
                | "royalBlue"
                | "saddleBrown"
                | "salmon"
                | "sandyBrown"
                | "seaGreen"
                | "seaShell"
                | "sienna"
                | "silver"
                | "skyBlue"
                | "slateBlue"
                | "slateGray"
                | "snow"
                | "springGreen"
                | "steelBlue"
                | "tan"
                | "teal"
                | "thistle"
                | "tomato"
                | "turquoise"
                | "violet"
                | "wheat"
                | "white"
                | "whiteSmoke"
                | "yellow"
                | "yellowGreen"
                | "darkBlue"
                | "darkCyan"
                | "darkGoldenrod"
                | "darkGray"
                | "darkGrey"
                | "darkGreen"
                | "darkKhaki"
                | "darkMagenta"
                | "darkOliveGreen"
                | "darkOrange"
                | "darkOrchid"
                | "darkRed"
                | "darkSalmon"
                | "darkSeaGreen"
                | "darkSlateBlue"
                | "darkSlateGray"
                | "darkSlateGrey"
                | "darkTurquoise"
                | "darkViolet"
                | "lightBlue"
                | "lightCoral"
                | "lightCyan"
                | "lightGoldenrodYellow"
                | "lightGray"
                | "lightGrey"
                | "lightGreen"
                | "lightPink"
                | "lightSalmon"
                | "lightSeaGreen"
                | "lightSkyBlue"
                | "lightSlateGray"
                | "lightSlateGrey"
                | "lightSteelBlue"
                | "lightYellow"
                | "mediumAquamarine"
                | "mediumBlue"
                | "mediumOrchid"
                | "mediumPurple"
                | "mediumSeaGreen"
                | "mediumSlateBlue"
                | "mediumSpringGreen"
                | "mediumTurquoise"
                | "mediumVioletRed"
                | "dkGrey"
                | "dimGrey"
                | "dkSlateGrey"
                | "grey"
                | "ltGrey"
                | "ltSlateGrey"
                | "slateGrey"
        ),
        (A, "prstDash", "val") => matches!(
            value,
            "solid"
                | "dot"
                | "dash"
                | "lgDash"
                | "dashDot"
                | "lgDashDot"
                | "lgDashDotDot"
                | "sysDash"
                | "sysDot"
                | "sysDashDot"
                | "sysDashDotDot"
        ),
        (A, "prstGeom", "prst") => matches!(
            value,
            "line"
                | "lineInv"
                | "triangle"
                | "rtTriangle"
                | "rect"
                | "diamond"
                | "parallelogram"
                | "trapezoid"
                | "nonIsoscelesTrapezoid"
                | "pentagon"
                | "hexagon"
                | "heptagon"
                | "octagon"
                | "decagon"
                | "dodecagon"
                | "star4"
                | "star5"
                | "star6"
                | "star7"
                | "star8"
                | "star10"
                | "star12"
                | "star16"
                | "star24"
                | "star32"
                | "roundRect"
                | "round1Rect"
                | "round2SameRect"
                | "round2DiagRect"
                | "snipRoundRect"
                | "snip1Rect"
                | "snip2SameRect"
                | "snip2DiagRect"
                | "plaque"
                | "ellipse"
                | "teardrop"
                | "homePlate"
                | "chevron"
                | "pieWedge"
                | "pie"
                | "blockArc"
                | "donut"
                | "noSmoking"
                | "rightArrow"
                | "leftArrow"
                | "upArrow"
                | "downArrow"
                | "stripedRightArrow"
                | "notchedRightArrow"
                | "bentUpArrow"
                | "leftRightArrow"
                | "upDownArrow"
                | "leftUpArrow"
                | "leftRightUpArrow"
                | "quadArrow"
                | "leftArrowCallout"
                | "rightArrowCallout"
                | "upArrowCallout"
                | "downArrowCallout"
                | "leftRightArrowCallout"
                | "upDownArrowCallout"
                | "quadArrowCallout"
                | "bentArrow"
                | "uturnArrow"
                | "circularArrow"
                | "leftCircularArrow"
                | "leftRightCircularArrow"
                | "curvedRightArrow"
                | "curvedLeftArrow"
                | "curvedUpArrow"
                | "curvedDownArrow"
                | "swooshArrow"
                | "cube"
                | "can"
                | "lightningBolt"
                | "heart"
                | "sun"
                | "moon"
                | "smileyFace"
                | "irregularSeal1"
                | "irregularSeal2"
                | "foldedCorner"
                | "bevel"
                | "frame"
                | "halfFrame"
                | "corner"
                | "diagStripe"
                | "chord"
                | "arc"
                | "leftBracket"
                | "rightBracket"
                | "leftBrace"
                | "rightBrace"
                | "bracketPair"
                | "bracePair"
                | "straightConnector1"
                | "bentConnector2"
                | "bentConnector3"
                | "bentConnector4"
                | "bentConnector5"
                | "curvedConnector2"
                | "curvedConnector3"
                | "curvedConnector4"
                | "curvedConnector5"
                | "callout1"
                | "callout2"
                | "callout3"
                | "accentCallout1"
                | "accentCallout2"
                | "accentCallout3"
                | "borderCallout1"
                | "borderCallout2"
                | "borderCallout3"
                | "accentBorderCallout1"
                | "accentBorderCallout2"
                | "accentBorderCallout3"
                | "wedgeRectCallout"
                | "wedgeRoundRectCallout"
                | "wedgeEllipseCallout"
                | "cloudCallout"
                | "cloud"
                | "ribbon"
                | "ribbon2"
                | "ellipseRibbon"
                | "ellipseRibbon2"
                | "leftRightRibbon"
                | "verticalScroll"
                | "horizontalScroll"
                | "wave"
                | "doubleWave"
                | "plus"
                | "flowChartProcess"
                | "flowChartDecision"
                | "flowChartInputOutput"
                | "flowChartPredefinedProcess"
                | "flowChartInternalStorage"
                | "flowChartDocument"
                | "flowChartMultidocument"
                | "flowChartTerminator"
                | "flowChartPreparation"
                | "flowChartManualInput"
                | "flowChartManualOperation"
                | "flowChartConnector"
                | "flowChartPunchedCard"
                | "flowChartPunchedTape"
                | "flowChartSummingJunction"
                | "flowChartOr"
                | "flowChartCollate"
                | "flowChartSort"
                | "flowChartExtract"
                | "flowChartMerge"
                | "flowChartOfflineStorage"
                | "flowChartOnlineStorage"
                | "flowChartMagneticTape"
                | "flowChartMagneticDisk"
                | "flowChartMagneticDrum"
                | "flowChartDisplay"
                | "flowChartDelay"
                | "flowChartAlternateProcess"
                | "flowChartOffpageConnector"
                | "actionButtonBlank"
                | "actionButtonHome"
                | "actionButtonHelp"
                | "actionButtonInformation"
                | "actionButtonForwardNext"
                | "actionButtonBackPrevious"
                | "actionButtonEnd"
                | "actionButtonBeginning"
                | "actionButtonReturn"
                | "actionButtonDocument"
                | "actionButtonSound"
                | "actionButtonMovie"
                | "gear6"
                | "gear9"
                | "funnel"
                | "mathPlus"
                | "mathMinus"
                | "mathMultiply"
                | "mathDivide"
                | "mathEqual"
                | "mathNotEqual"
                | "cornerTabs"
                | "squareTabs"
                | "plaqueTabs"
                | "chartX"
                | "chartStar"
                | "chartPlus"
        ),
        (A, "prstShdw", "prst") => matches!(
            value,
            "shdw1"
                | "shdw2"
                | "shdw3"
                | "shdw4"
                | "shdw5"
                | "shdw6"
                | "shdw7"
                | "shdw8"
                | "shdw9"
                | "shdw10"
                | "shdw11"
                | "shdw12"
                | "shdw13"
                | "shdw14"
                | "shdw15"
                | "shdw16"
                | "shdw17"
                | "shdw18"
                | "shdw19"
                | "shdw20"
        ),
        (A, "prstShdw", "dist") => value.parse::<i64>().is_ok(),
        (A, "prstShdw", "dir") => value.parse::<i32>().is_ok(),
        (A, "prstTxWarp", "prst") => matches!(
            value,
            "textNoShape"
                | "textPlain"
                | "textStop"
                | "textTriangle"
                | "textTriangleInverted"
                | "textChevron"
                | "textChevronInverted"
                | "textRingInside"
                | "textRingOutside"
                | "textArchUp"
                | "textArchDown"
                | "textCircle"
                | "textButton"
                | "textArchUpPour"
                | "textArchDownPour"
                | "textCirclePour"
                | "textButtonPour"
                | "textCurveUp"
                | "textCurveDown"
                | "textCanUp"
                | "textCanDown"
                | "textWave1"
                | "textWave2"
                | "textDoubleWave1"
                | "textWave4"
                | "textInflate"
                | "textDeflate"
                | "textInflateBottom"
                | "textDeflateBottom"
                | "textInflateTop"
                | "textDeflateTop"
                | "textDeflateInflate"
                | "textDeflateInflateDeflate"
                | "textFadeRight"
                | "textFadeLeft"
                | "textFadeUp"
                | "textFadeDown"
                | "textSlantUp"
                | "textSlantDown"
                | "textCascadeUp"
                | "textCascadeDown"
        ),
        (A, "reflection", "blurRad") => value.parse::<i64>().is_ok(),
        (A, "reflection", "stA") => value.parse::<i32>().is_ok(),
        (A, "reflection", "stPos") => value.parse::<i32>().is_ok(),
        (A, "reflection", "endA") => value.parse::<i32>().is_ok(),
        (A, "reflection", "endPos") => value.parse::<i32>().is_ok(),
        (A, "reflection", "dist") => value.parse::<i64>().is_ok(),
        (A, "reflection", "dir") => value.parse::<i32>().is_ok(),
        (A, "reflection", "fadeDir") => value.parse::<i32>().is_ok(),
        (A, "reflection", "sx") => value.parse::<i32>().is_ok(),
        (A, "reflection", "sy") => value.parse::<i32>().is_ok(),
        (A, "reflection", "kx") => value.parse::<i32>().is_ok(),
        (A, "reflection", "ky") => value.parse::<i32>().is_ok(),
        (A, "reflection", "algn") => matches!(
            value,
            "tl" | "t" | "tr" | "l" | "ctr" | "r" | "bl" | "b" | "br"
        ),
        (A, "reflection", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "relOff", "tx") => value.parse::<i32>().is_ok(),
        (A, "relOff", "ty") => value.parse::<i32>().is_ok(),
        (A, "schemeClr", "val") => matches!(
            value,
            "bg1"
                | "tx1"
                | "bg2"
                | "tx2"
                | "accent1"
                | "accent2"
                | "accent3"
                | "accent4"
                | "accent5"
                | "accent6"
                | "hlink"
                | "folHlink"
                | "phClr"
                | "dk1"
                | "lt1"
                | "dk2"
                | "lt2"
        ),
        (A, "scrgbClr", "r") => value.parse::<i32>().is_ok(),
        (A, "scrgbClr", "g") => value.parse::<i32>().is_ok(),
        (A, "scrgbClr", "b") => value.parse::<i32>().is_ok(),
        (A, "softEdge", "rad") => value.parse::<i64>().is_ok(),
        (A, "spLocks", "noGrp") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noSelect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noRot") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noChangeAspect") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noMove") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noResize") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noEditPoints") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noAdjustHandles") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noChangeArrowheads") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noChangeShapeType") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spLocks", "noTextEdit") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "spPr", "bwMode") => matches!(
            value,
            "clr"
                | "auto"
                | "gray"
                | "ltGray"
                | "invGray"
                | "grayWhite"
                | "blackGray"
                | "blackWhite"
                | "black"
                | "white"
                | "hidden"
        ),
        (A, "spcPct", "val") => value.parse::<i32>().is_ok(),
        (A, "spcPts", "val") => value.parse::<i32>().is_ok(),
        (A, "tab", "pos") => value.parse::<i32>().is_ok(),
        (A, "tab", "algn") => matches!(value, "l" | "ctr" | "r" | "dec"),
        (A, "tc", "rowSpan") => value.parse::<i32>().is_ok(),
        (A, "tc", "gridSpan") => value.parse::<i32>().is_ok(),
        (A, "tc", "hMerge") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "tc", "vMerge") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "tcPr", "marL") => value.parse::<i32>().is_ok(),
        (A, "tcPr", "marR") => value.parse::<i32>().is_ok(),
        (A, "tcPr", "marT") => value.parse::<i32>().is_ok(),
        (A, "tcPr", "marB") => value.parse::<i32>().is_ok(),
        (A, "tcPr", "vert") => matches!(
            value,
            "horz"
                | "vert"
                | "vert270"
                | "wordArtVert"
                | "eaVert"
                | "mongolianVert"
                | "wordArtVertRtl"
        ),
        (A, "tcPr", "anchor") => matches!(value, "t" | "ctr" | "b"),
        (A, "tcPr", "anchorCtr") => matches!(value, "true" | "false" | "0" | "1"),
        (A, "tcPr", "horzOverflow") => matches!(value, "overflow" | "clip"),
        (A, "tile", "tx") => value.parse::<i64>().is_ok(),
        (A, "tile", "ty") => value.parse::<i64>().is_ok(),
        (A, "tile", "sx") => value.parse::<i32>().is_ok(),
        (A, "tile", "sy") => value.parse::<i32>().is_ok(),
        (A, "tile", "flip") => matches!(value, "none" | "x" | "y" | "xy"),
        (A, "tile", "algn") => matches!(
            value,
            "tl" | "t" | "tr" | "l" | "ctr" | "r" | "bl" | "b" | "br"
        ),
        (A, "tr", "h") => value.parse::<i64>().is_ok(),
        (P, "blipFill", "dpi") => value.parse::<u32>().is_ok(),
        (P, "blipFill", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (P, "cNvPicPr", "preferRelativeResize") => matches!(value, "true" | "false" | "0" | "1"),
        (P, "notes", "showMasterSp") => matches!(value, "true" | "false" | "0" | "1"),
        (P, "notes", "showMasterPhAnim") => matches!(value, "true" | "false" | "0" | "1"),
        (P, "spPr", "bwMode") => matches!(
            value,
            "clr"
                | "auto"
                | "gray"
                | "ltGray"
                | "invGray"
                | "grayWhite"
                | "blackGray"
                | "blackWhite"
                | "black"
                | "white"
                | "hidden"
        ),
        (PIC, "blipFill", "dpi") => value.parse::<u32>().is_ok(),
        (PIC, "blipFill", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (PIC, "cNvPicPr", "preferRelativeResize") => matches!(value, "true" | "false" | "0" | "1"),
        (PIC, "spPr", "bwMode") => matches!(
            value,
            "clr"
                | "auto"
                | "gray"
                | "ltGray"
                | "invGray"
                | "grayWhite"
                | "blackGray"
                | "blackWhite"
                | "black"
                | "white"
                | "hidden"
        ),
        (S, "horizontal", "style") => matches!(
            value,
            "none"
                | "thin"
                | "medium"
                | "dashed"
                | "dotted"
                | "thick"
                | "double"
                | "hair"
                | "mediumDashed"
                | "dashDot"
                | "mediumDashDot"
                | "dashDotDot"
                | "mediumDashDotDot"
                | "slantDashDot"
        ),
        (S, "vertical", "style") => matches!(
            value,
            "none"
                | "thin"
                | "medium"
                | "dashed"
                | "dotted"
                | "thick"
                | "double"
                | "hair"
                | "mediumDashed"
                | "dashDot"
                | "mediumDashDot"
                | "dashDotDot"
                | "mediumDashDotDot"
                | "slantDashDot"
        ),
        (S, "workbookPr", "date1904") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "dateCompatibility") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "showObjects") => matches!(value, "all" | "placeholders" | "none"),
        (S, "workbookPr", "showBorderUnselectedTables") => {
            matches!(value, "true" | "false" | "0" | "1")
        }
        (S, "workbookPr", "filterPrivacy") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "showInkAnnotation") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "hidePivotFieldList") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "showPivotChartFilter") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "autoCompressPictures") => matches!(value, "true" | "false" | "0" | "1"),
        (S, "workbookPr", "defaultThemeVersion") => value.parse::<u32>().is_ok(),
        (W, "bdo", "val") => matches!(value, "ltr" | "rtl"),
        (W, "dir", "val") => matches!(value, "ltr" | "rtl"),
        (W, "hMerge", "val") => matches!(value, "continue" | "restart"),
        (W, "hideMark", "val") => matches!(value, "on" | "off"),
        (W, "noEndnote", "val") => matches!(value, "true" | "false" | "0" | "1" | "on" | "off"),
        (W, "noWrap", "val") => matches!(value, "on" | "off"),
        (W, "pgBorders", "zOrder") => matches!(value, "front" | "back"),
        (W, "pgBorders", "display") => matches!(value, "allPages" | "firstPage" | "notFirstPage"),
        (W, "pgBorders", "offsetFrom") => matches!(value, "page" | "text"),
        (W, "ptab", "alignment") => matches!(value, "left" | "center" | "right"),
        (W, "ptab", "relativeTo") => matches!(value, "margin" | "indent"),
        (W, "ptab", "leader") => matches!(
            value,
            "none" | "dot" | "hyphen" | "underscore" | "middleDot"
        ),
        (W, "tblInd", "w") => value.parse::<i32>().is_ok(),
        (W, "tblInd", "type") => matches!(value, "nil" | "pct" | "dxa" | "auto"),
        (W, "tblLayout", "type") => matches!(value, "fixed" | "autofit"),
        (W, "tblOverlap", "val") => matches!(value, "never" | "overlap"),
        (W, "tblpPr", "leftFromText") => value.parse::<i16>().is_ok(),
        (W, "tblpPr", "rightFromText") => value.parse::<i16>().is_ok(),
        (W, "tblpPr", "topFromText") => value.parse::<i16>().is_ok(),
        (W, "tblpPr", "bottomFromText") => value.parse::<i16>().is_ok(),
        (W, "tblpPr", "vertAnchor") => matches!(value, "text" | "margin" | "page"),
        (W, "tblpPr", "horzAnchor") => matches!(value, "text" | "margin" | "page"),
        (W, "tblpPr", "tblpXSpec") => {
            matches!(value, "left" | "center" | "right" | "inside" | "outside")
        }
        (W, "tblpPr", "tblpX") => value.parse::<i32>().is_ok(),
        (W, "tblpPr", "tblpYSpec") => matches!(
            value,
            "inline" | "top" | "center" | "bottom" | "inside" | "outside"
        ),
        (W, "tblpPr", "tblpY") => value.parse::<i32>().is_ok(),
        (W, "textAlignment", "val") => {
            matches!(value, "top" | "center" | "baseline" | "bottom" | "auto")
        }
        (W, "textboxTightWrap", "val") => matches!(
            value,
            "none" | "allLines" | "firstAndLastLine" | "firstLineOnly" | "lastLineOnly"
        ),
        (W, "trHeight", "val") => value.parse::<u32>().is_ok(),
        (W, "trHeight", "hRule") => matches!(value, "auto" | "exact" | "atLeast"),
        (W, "vMerge", "val") => matches!(value, "continue" | "restart"),
        (XDR, "blipFill", "rotWithShape") => matches!(value, "true" | "false" | "0" | "1"),
        (XDR, "cNvPicPr", "preferRelativeResize") => matches!(value, "true" | "false" | "0" | "1"),
        (XDR, "spPr", "bwMode") => matches!(
            value,
            "clr"
                | "auto"
                | "gray"
                | "ltGray"
                | "invGray"
                | "grayWhite"
                | "blackGray"
                | "blackWhite"
                | "black"
                | "white"
                | "hidden"
        ),
        _ => false,
    }
}
