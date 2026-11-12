//! Explicit additional styling rules for OOXML allowlist v1.
//! Element/attribute names checked against the MIT-licensed Microsoft Open XML
//! SDK schema data at 431ab05cf160248cc3885a4a766026d4f8243792.
//! This is a selected static subset; unknown elements and attributes still refuse.
use super::{A, P, S, W};
pub(super) fn rule(ns: &str, element: &str) -> Option<&'static str> {
    Some(match (ns,element){
        (W,"fonts")=>"",
        (W,"font")=>"name",
        (W,"altName")=>"val",
        (W,"panose1")=>"val",
        (W,"charset")=>"characterSet val",
        (W,"family")=>"val",
        (W,"notTrueType")=>"val",
        (W,"pitch")=>"val",
        (W,"sig")=>"csb0 csb1 usb0 usb1 usb2 usb3",
        (W,"compat")=>"",
        (W,"compatSetting")=>"name uri val",
        (W,"latentStyles")=>"count defLockedState defQFormat defSemiHidden defUIPriority defUnhideWhenUsed",
        (W,"lsdException")=>"locked name qFormat semiHidden uiPriority unhideWhenUsed",
        (W,"tabs")=>"",
        (W,"kern")=>"val",
        (W,"position")=>"val",
        (W,"bCs")=>"val",
        (W,"iCs")=>"val",
        (W,"snapToGrid")=>"val",
        (W,"rtl")=>"val",
        (W,"cs")=>"val",
        (W,"em")=>"val",
        (W,"semiHidden")=>"val",
        (W,"unhideWhenUsed")=>"val",
        (W,"autoRedefine")=>"val",
        (W,"personal")=>"val",
        (W,"personalCompose")=>"val",
        (W,"personalReply")=>"val",
        (W,"locked")=>"val",
        (W,"nsid")=>"val",
        (W,"tmpl")=>"val",
        (W,"startOverride")=>"val",
        (W,"lvlRestart")=>"val",
        (W,"suff")=>"val",
        (W,"isLgl")=>"val",
        (W,"lvlPicBulletId")=>"val",
        (W,"pgNumType")=>"chapSep chapStyle fmt start",
        (W,"type")=>"val",
        (W,"formProt")=>"val",
        (W,"proofState")=>"grammar spelling",
        (W,"themeFontLang")=>"bidi eastAsia val",
        (W,"clrSchemeMapping")=>"accent1 accent2 accent3 accent4 accent5 accent6 bg1 bg2 followedHyperlink hyperlink t1 t2",
        (W,"decimalSymbol")=>"val",
        (W,"listSeparator")=>"val",
        (W,"view")=>"val",
        (W,"mirrorMargins")=>"val",
        (W,"autoHyphenation")=>"val",
        (W,"consecutiveHyphenLimit")=>"val",
        (W,"hyphenationZone")=>"val",
        (W,"doNotHyphenateCaps")=>"val",
        (W,"gutterAtTop")=>"val",
        (W,"defaultTableStyle")=>"val",
        (W,"doNotTrackMoves")=>"val",
        (W,"doNotTrackFormatting")=>"val",
        (W,"doNotUseHTMLParagraphAutoSpacing")=>"val",
        (A,"gradFill")=>"flip rotWithShape",
        (A,"gsLst")=>"",
        (A,"gs")=>"pos",
        (A,"lin")=>"ang scaled",
        (A,"path")=>"extrusionOk fill h path stroke w",
        (A,"tileRect")=>"b l r t",
        (A,"outerShdw")=>"algn blurRad dir dist kx ky rotWithShape sx sy",
        (A,"innerShdw")=>"blurRad dir dist",
        (A,"softEdge")=>"rad",
        (A,"glow")=>"rad",
        (A,"reflection")=>"algn blurRad dir dist endA endPos fadeDir kx ky rotWithShape stA stPos sx sy",
        (A,"blur")=>"grow rad",
        (A,"scene3d")=>"",
        (A,"camera")=>"fov prst zoom",
        (A,"lightRig")=>"dir rig",
        (A,"rot")=>"lat lon rev",
        (A,"sp3d")=>"contourW extrusionH prstMaterial z",
        (A,"bevelT")=>"h prst w",
        (A,"bevelB")=>"h prst w",
        (A,"contourClr")=>"",
        (A,"extrusionClr")=>"",
        (A,"flatTx")=>"z",
        (A,"defPPr")=>"algn defTabSz eaLnBrk fontAlgn hangingPunct indent latinLnBrk lvl marL marR rtl",
        (A,"lnSpc")=>"",
        (A,"spcBef")=>"",
        (A,"spcAft")=>"",
        (A,"spcPct")=>"val",
        (A,"spcPts")=>"val",
        (A,"satMod")=>"val",
        (A,"satOff")=>"val",
        (A,"sat")=>"val",
        (A,"hue")=>"val",
        (A,"hueMod")=>"val",
        (A,"hueOff")=>"val",
        (A,"lum")=>"bright contrast val",
        (A,"red")=>"val",
        (A,"redMod")=>"val",
        (A,"redOff")=>"val",
        (A,"green")=>"val",
        (A,"greenMod")=>"val",
        (A,"greenOff")=>"val",
        (A,"blue")=>"val",
        (A,"blueMod")=>"val",
        (A,"blueOff")=>"val",
        (A,"alphaMod")=>"val",
        (A,"alphaOff")=>"val",
        (A,"gamma")=>"",
        (A,"invGamma")=>"",
        (A,"gray")=>"",
        (A,"inv")=>"",
        (A,"comp")=>"",
        (A,"prstClr")=>"val",
        (A,"scrgbClr")=>"b g r",
        (A,"hslClr")=>"hue lum sat",
        (A,"custClrLst")=>"",
        (A,"custClr")=>"name",
        (A,"spDef")=>"",
        (A,"lnDef")=>"",
        (A,"txDef")=>"",
        (A,"tblStyleLst")=>"def",
        (A,"tblStyle")=>"styleId styleName",
        (A,"tblBg")=>"",
        (A,"wholeTbl")=>"",
        (A,"band1H")=>"",
        (A,"band2H")=>"",
        (A,"band1V")=>"",
        (A,"band2V")=>"",
        (A,"lastCol")=>"",
        (A,"firstCol")=>"",
        (A,"lastRow")=>"",
        (A,"seCell")=>"",
        (A,"swCell")=>"",
        (A,"firstRow")=>"",
        (A,"neCell")=>"",
        (A,"nwCell")=>"",
        (A,"tcStyle")=>"",
        (A,"tcTxStyle")=>"b i",
        (A,"tcBdr")=>"",
        (A,"insideH")=>"",
        (A,"insideV")=>"",
        (A,"tl2br")=>"",
        (A,"tr2bl")=>"",
        (A,"round")=>"",
        (A,"bevel")=>"h prst w",
        (A,"miter")=>"lim",
        (A,"solidFill")=>"",
        (P,"bg")=>"bwMode",
        (P,"bgPr")=>"shadeToTitle",
        (P,"bgRef")=>"idx",
        (S,"vertAlign")=>"val",
        (S,"outline")=>"val",
        (S,"shadow")=>"val",
        (S,"condense")=>"val",
        (S,"extend")=>"val",
        (S,"tabColor")=>"auto indexed rgb theme tint",
        (A,"tbl")=>"",
        (A,"tblPr")=>"bandCol bandRow firstCol firstRow lastCol lastRow rtl",
        (A,"tblGrid")=>"",
        (A,"gridCol")=>"w",
        (A,"tr")=>"h",
        (A,"tc")=>"gridSpan hMerge rowSpan vMerge",
        (A,"tcPr")=>"anchor anchorCtr horzOverflow marB marL marR marT vert",
        (A,"tableStyleId")=>"",
        (P,"graphicFrame")=>"",
        (P,"nvGraphicFramePr")=>"",
        (P,"cNvGraphicFramePr")=>"",
        (P,"xfrm")=>"flipH flipV rot",
        (P,"presentationPr")=>"",
        (P,"showPr")=>"loop showAnimation showNarration useTimings",
        (P,"present")=>"",
        (P,"browse")=>"showScrollbar",
        (P,"kiosk")=>"restart",
        (P,"sldAll")=>"",
        (P,"sldRg")=>"end st",
        (W,"removePersonalInformation")=>"val",
        (W,"removeDateAndTime")=>"val",
        (W,"doNotDisplayPageBoundaries")=>"val",
        (W,"displayBackgroundShape")=>"val",
        (W,"printFractionalCharacterWidth")=>"val",
        (W,"alignBordersAndEdges")=>"val",
        (W,"bordersDoNotSurroundHeader")=>"val",
        (W,"bordersDoNotSurroundFooter")=>"val",
        (W,"hideSpellingErrors")=>"val",
        (W,"hideGrammaticalErrors")=>"val",
        (W,"stylePaneFormatFilter")=>"allStyles alternateStyleNames clearFormatting customStyles directFormattingOnNumbering directFormattingOnParagraphs directFormattingOnRuns directFormattingOnTables headingStyles latentStyles numberingStyles stylesInUse tableStyles top3HeadingStyles val visibleStyles",
        (W,"stylePaneSortMethod")=>"val",
        (W,"defaultTabStop")=>"val",
        (W,"clickAndTypeStyle")=>"val",
        (W,"evenAndOddHeaders")=>"val",
        (W,"bookFoldRevPrinting")=>"val",
        (W,"bookFoldPrinting")=>"val",
        (W,"bookFoldPrintingSheets")=>"val",
        (W,"drawingGridHorizontalSpacing")=>"val",
        (W,"drawingGridVerticalSpacing")=>"val",
        (W,"displayHorizontalDrawingGridEvery")=>"val",
        (W,"displayVerticalDrawingGridEvery")=>"val",
        (W,"doNotUseMarginsForDrawingGridOrigin")=>"val",
        (W,"drawingGridHorizontalOrigin")=>"val",
        (W,"drawingGridVerticalOrigin")=>"val",
        (W,"noPunctuationKerning")=>"val",
        (W,"characterSpacingControl")=>"val",
        (W,"strictFirstAndLastChars")=>"val",
        (W,"footnotePr")=>"",
        (W,"endnotePr")=>"",
        (W,"doNotAutoCompressPictures")=>"val",
        (W,"tblStyle")=>"val",
        (W,"tblLook")=>"firstColumn firstRow lastColumn lastRow noHBand noVBand val",
        (W,"tblStylePr")=>"type",
        (W,"tblInd")=>"type w",
        (W,"tblCellMar")=>"",
        (W,"tblCellSpacing")=>"type w",
        (W,"trHeight")=>"hRule val",
        (W,"textAlignment")=>"val",
        (W,"suppressAutoHyphens")=>"val",
        (W,"suppressLineNumbers")=>"val",
        (W,"adjustRightInd")=>"val",
        (W,"autoSpaceDE")=>"val",
        (W,"autoSpaceDN")=>"val",
        (W,"bidi")=>"val",
        (W,"numPr")=>"",
        (W,"pBdr")=>"",
        (W,"between")=>"color frame shadow space sz themeColor themeShade themeTint val",
        (W,"bar")=>"color frame shadow space sz themeColor themeShade themeTint val",
        (W,"separator")=>"",
        (W,"continuationSeparator")=>"",
        (W,"pos")=>"val",
        (W,"numStart")=>"val",
        (W,"numRestart")=>"val",
        (W,"useSingleBorderforContiguousCells")=>"val",
        (W,"wpJustification")=>"val",
        (W,"noTabHangInd")=>"val",
        (W,"noLeading")=>"val",
        (W,"spaceForUL")=>"val",
        (W,"noColumnBalance")=>"val",
        (W,"balanceSingleByteDoubleByteWidth")=>"val",
        (W,"noExtraLineSpacing")=>"val",
        (W,"doNotLeaveBackslashAlone")=>"val",
        (W,"ulTrailSpace")=>"val",
        (W,"doNotExpandShiftReturn")=>"val",
        (W,"spacingInWholePoints")=>"val",
        (W,"lineWrapLikeWord6")=>"val",
        (W,"printBodyTextBeforeHeader")=>"val",
        (W,"printColBlack")=>"val",
        (W,"wpSpaceWidth")=>"val",
        (W,"showBreaksInFrames")=>"val",
        (W,"subFontBySize")=>"val",
        (W,"suppressBottomSpacing")=>"val",
        (W,"suppressTopSpacing")=>"val",
        (W,"suppressSpacingAtTopOfPage")=>"val",
        (W,"suppressTopSpacingWP")=>"val",
        (W,"suppressSpBfAfterPgBrk")=>"val",
        (W,"swapBordersFacingPages")=>"val",
        (W,"convMailMergeEsc")=>"val",
        (W,"truncateFontHeightsLikeWP6")=>"val",
        (W,"mwSmallCaps")=>"val",
        (W,"usePrinterMetrics")=>"val",
        (W,"doNotSuppressParagraphBorders")=>"val",
        (W,"wrapTrailSpaces")=>"val",
        (W,"footnoteLayoutLikeWW8")=>"val",
        (W,"shapeLayoutLikeWW8")=>"val",
        (W,"alignTablesRowByRow")=>"val",
        (W,"forgetLastTabAlignment")=>"val",
        (W,"adjustLineHeightInTable")=>"val",
        (W,"autoSpaceLikeWord95")=>"val",
        (W,"noSpaceRaiseLower")=>"val",
        (W,"layoutRawTableWidth")=>"val",
        (W,"layoutTableRowsApart")=>"val",
        (W,"useWord97LineBreakRules")=>"val",
        (W,"doNotBreakWrappedTables")=>"val",
        (W,"doNotSnapToGridInCell")=>"val",
        (W,"selectFldWithFirstOrLastChar")=>"val",
        (W,"applyBreakingRules")=>"val",
        (W,"doNotWrapTextWithPunct")=>"val",
        (W,"doNotUseEastAsianBreakRules")=>"val",
        (W,"useWord2002TableStyleRules")=>"val",
        (W,"growAutofit")=>"val",
        (W,"useFELayout")=>"val",
        (W,"useNormalStyleForList")=>"val",
        (W,"doNotUseIndentAsNumberingTabStop")=>"val",
        (W,"useAltKinsokuLineBreakRules")=>"val",
        (W,"allowSpaceOfSameStyleInTable")=>"val",
        (W,"doNotSuppressIndentation")=>"val",
        (W,"doNotAutofitConstrainedTables")=>"val",
        (W,"autofitToFirstFixedWidthCell")=>"val",
        (W,"underlineTabInNumList")=>"val",
        (W,"displayHangulFixedWidth")=>"val",
        (W,"splitPgBreakAndParaMark")=>"val",
        (W,"doNotVertAlignCellWithSp")=>"val",
        (W,"doNotBreakConstrainedForcedTable")=>"val",
        (W,"doNotVertAlignInTxbx")=>"val",
        (W,"useAnsiKerningPairs")=>"val",
        (W,"cachedColBalance")=>"val",
        (A,"fillToRect")=>"b l r t",
        (A,"tabLst")=>"",
        (A,"tab")=>"algn pos",
        (A,"buSzPct")=>"val",
        (A,"buSzPts")=>"val",
        (A,"buSzTx")=>"",
        (A,"buFontTx")=>"",
        (A,"buClr")=>"",
        (A,"buClrTx")=>"",
        (A,"lnL")=>"algn cap cmpd w",
        (A,"lnR")=>"algn cap cmpd w",
        (A,"lnT")=>"algn cap cmpd w",
        (A,"lnB")=>"algn cap cmpd w",
        (A,"cell3D")=>"prstMaterial",
        (A,"noFill")=>"",
        _=>return None,
    })
}

// Rendering compatibility flags contain only a schema-defined boolean value.
// Keep these explicit: mail merge, field updates, transforms and arbitrary
// editor settings are never admitted by the settings-part rule.
pub(super) fn settings_element(element: &str) -> bool {
    matches!(
        element,
        "removePersonalInformation"
            | "removeDateAndTime"
            | "doNotDisplayPageBoundaries"
            | "displayBackgroundShape"
            | "printFractionalCharacterWidth"
            | "mirrorMargins"
            | "alignBordersAndEdges"
            | "bordersDoNotSurroundHeader"
            | "bordersDoNotSurroundFooter"
            | "hideSpellingErrors"
            | "hideGrammaticalErrors"
            | "stylePaneFormatFilter"
            | "stylePaneSortMethod"
            | "defaultTabStop"
            | "autoHyphenation"
            | "consecutiveHyphenLimit"
            | "hyphenationZone"
            | "doNotHyphenateCaps"
            | "clickAndTypeStyle"
            | "defaultTableStyle"
            | "evenAndOddHeaders"
            | "bookFoldRevPrinting"
            | "bookFoldPrinting"
            | "bookFoldPrintingSheets"
            | "drawingGridHorizontalSpacing"
            | "drawingGridVerticalSpacing"
            | "displayHorizontalDrawingGridEvery"
            | "displayVerticalDrawingGridEvery"
            | "doNotUseMarginsForDrawingGridOrigin"
            | "drawingGridHorizontalOrigin"
            | "drawingGridVerticalOrigin"
            | "noPunctuationKerning"
            | "characterSpacingControl"
            | "strictFirstAndLastChars"
            | "footnotePr"
            | "endnotePr"
            | "doNotAutoCompressPictures"
            | "useSingleBorderforContiguousCells"
            | "wpJustification"
            | "noTabHangInd"
            | "noLeading"
            | "spaceForUL"
            | "noColumnBalance"
            | "balanceSingleByteDoubleByteWidth"
            | "noExtraLineSpacing"
            | "doNotLeaveBackslashAlone"
            | "ulTrailSpace"
            | "doNotExpandShiftReturn"
            | "spacingInWholePoints"
            | "lineWrapLikeWord6"
            | "printBodyTextBeforeHeader"
            | "printColBlack"
            | "wpSpaceWidth"
            | "showBreaksInFrames"
            | "subFontBySize"
            | "suppressBottomSpacing"
            | "suppressTopSpacing"
            | "suppressSpacingAtTopOfPage"
            | "suppressTopSpacingWP"
            | "suppressSpBfAfterPgBrk"
            | "swapBordersFacingPages"
            | "convMailMergeEsc"
            | "truncateFontHeightsLikeWP6"
            | "mwSmallCaps"
            | "usePrinterMetrics"
            | "doNotSuppressParagraphBorders"
            | "wrapTrailSpaces"
            | "footnoteLayoutLikeWW8"
            | "shapeLayoutLikeWW8"
            | "alignTablesRowByRow"
            | "forgetLastTabAlignment"
            | "adjustLineHeightInTable"
            | "autoSpaceLikeWord95"
            | "noSpaceRaiseLower"
            | "doNotUseHTMLParagraphAutoSpacing"
            | "layoutRawTableWidth"
            | "layoutTableRowsApart"
            | "useWord97LineBreakRules"
            | "doNotBreakWrappedTables"
            | "doNotSnapToGridInCell"
            | "selectFldWithFirstOrLastChar"
            | "applyBreakingRules"
            | "doNotWrapTextWithPunct"
            | "doNotUseEastAsianBreakRules"
            | "useWord2002TableStyleRules"
            | "growAutofit"
            | "useFELayout"
            | "useNormalStyleForList"
            | "doNotUseIndentAsNumberingTabStop"
            | "useAltKinsokuLineBreakRules"
            | "allowSpaceOfSameStyleInTable"
            | "doNotSuppressIndentation"
            | "doNotAutofitConstrainedTables"
            | "autofitToFirstFixedWidthCell"
            | "underlineTabInNumList"
            | "displayHangulFixedWidth"
            | "splitPgBreakAndParaMark"
            | "doNotVertAlignCellWithSp"
            | "doNotBreakConstrainedForcedTable"
            | "doNotVertAlignInTxbx"
            | "useAnsiKerningPairs"
            | "cachedColBalance"
    )
}
