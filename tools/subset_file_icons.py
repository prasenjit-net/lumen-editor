#!/usr/bin/env python3
"""Build assets/file-icons.ttf, the explorer's file and folder icons.

The glyphs come from the Nerd Fonts "Symbols Only" font (MIT; Seti-UI,
Devicons and Material Design Icons sets inside it), subset to the ranges that
src/ui/file_icons.rs uses. Run with fontTools installed:

    curl -L -o /tmp/nf.zip https://github.com/ryanoasis/nerd-fonts/releases/download/v3.4.0/NerdFontsSymbolsOnly.zip
    unzip -o /tmp/nf.zip -d /tmp/nf
    python3 tools/subset_file_icons.py /tmp/nf/SymbolsNerdFontMono-Regular.ttf
"""
import pathlib, sys
from fontTools import subset

ROOT = pathlib.Path(__file__).resolve().parent.parent
UNICODES = (
    list(range(0xE5FA, 0xE6B8))  # Seti-UI + custom
    + list(range(0xE700, 0xE7C6))  # Devicons
    + [0xF0214, 0xF033E, 0xF018D, 0xF0626]  # file, lock, console, json
    + list(range(0xF024B, 0xF0260))  # folder variants
    + [0xF06EB, 0xF0770, 0xF07B9, 0xF0C82, 0xF107D, 0xF12E3, 0xF1359, 0xF0968, 0xF0870, 0xF08AC, 0xF10B5, 0xF10B7, 0xF069D, 0xF19F0, 0xF19FA, 0xF19FC]
)
opts = subset.Options()
opts.layout_features = []
opts.name_IDs = [0, 1, 2, 3, 4, 5, 6]
opts.notdef_outline = True
font = subset.load_font(sys.argv[1], opts)
sub = subset.Subsetter(opts)
sub.populate(unicodes=UNICODES)
sub.subset(font)
subset.save_font(font, ROOT / "assets/file-icons.ttf", opts)
