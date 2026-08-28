#!/usr/bin/env python3
"""Write the `.DS_Store` that gives the mounted image its window.

Finder keeps a folder's window frame, view style, and icon positions in a
`.DS_Store` inside that folder and nowhere else. A volume without one opens at
whatever size and view Finder last used for an unknown folder, sidebar and
toolbar showing: a file browser pointed at a disk, rather than the
drag-to-install panel a reader expects. Staging the file is the whole fix, and
it travels with `-srcfolder` like any other file in the image.

The other way to produce that layout is to drive Finder over Apple events, as
`create-dmg` does. It needs a GUI session, and on a CI runner it fails as a TCC
denial or an AppleEvent timeout, so it is not an option here. Nothing below
talks to Finder or to the network, and nothing has to be installed for it: the
interpreter and `plistlib` arrive with the Command Line Tools that build the
binary in the first place.

Only the records that window needs are written, all in one B-tree leaf:

    bwsp   the window frame, with toolbar, sidebar, and pathbar hidden
    icvp   icon view: icon size, text size, label under the icon, no arranging
    icvl   the view style, so the volume opens in icon view at all
    vSrn   a version marker Finder expects to find beside them
    Iloc   one per icon: where the app and the `Applications` link sit

The container is Apple's buddy allocator. A 36-byte header names a bookkeeping
block, which holds the block table, a name-to-block table, and the free lists.
A block address packs its own size: `offset | log2(size)`, with the offset
counted from byte 4 of the file rather than byte 0, so a block at address `a`
begins at file offset `a + 4`. Three blocks cover this file, each aligned to
its own size the way a buddy allocator would place it.

The free lists are written empty, which is the one liberty taken here. They
exist so a *writer* can find space again, and nothing writes this file twice:
it is built once and lands on a read-only image, where Finder cannot rewrite
it. Finder replaces a `.DS_Store` wholesale when it does change one, so there
is no partial-update path that would read them.
"""

import plistlib
import struct
import sys

# The window, in Finder's coordinates: a panel wide enough for two 128px icons
# with room between them, positioned near the top left of a small display.
WINDOW_X, WINDOW_Y = 400, 180
WINDOW_W, WINDOW_H = 560, 400

ICON_SIZE = 128
TEXT_SIZE = 12

# Icon centres, derived from the window rather than written out, so moving the
# window keeps the pair centred in it. The gap is centre to centre.
ICON_GAP = 260
ICON_Y = 180

PAGE_SIZE = 4096

# Blocks, as (address, size). Block 0 is the bookkeeping block the header
# names, 1 is the B-tree's superblock, 2 is its single leaf. Sizes are powers
# of two and each address is a multiple of its size.
ROOT_BLOCK = (0x2000, 2048)
DSDB_BLOCK = (0x20, 32)
NODE_BLOCK = (0x1000, PAGE_SIZE)

# Opaque to every reader of this format, and written back verbatim by the ones
# that rewrite it. Copied from a file the tools accept rather than zeroed.
HEADER_UNKNOWN = b"\x00\x00\x10\x0c\x00\x00\x00\x87\x00\x00\x20\x0b\x00\x00\x00\x00"


def address(block):
    """Pack a block's size into its address, which is how the table stores it."""
    offset, size = block
    return offset | (size.bit_length() - 1)


def record(name, code, typecode, value):
    """One leaf record: the name it describes, what it says, and how it is stored."""
    if typecode == b"blob":
        payload = struct.pack(b">I", len(value)) + value
    elif typecode == b"long":
        payload = struct.pack(b">I", value)
    elif typecode == b"type":
        payload = value
    else:
        raise ValueError(f"unsupported record type {typecode!r}")
    # The length is in UTF-16 code units, which is not the Python character
    # count for anything outside the BMP.
    utf16 = name.encode("utf-16be")
    return struct.pack(b">I", len(utf16) // 2) + utf16 + code + typecode + payload


def iloc(x, y):
    """An icon position. The trailing words are fixed and mean nothing here."""
    return struct.pack(b">IIII", x, y, 0xFFFFFFFF, 0xFFFF0000)


def plist(mapping):
    return plistlib.dumps(mapping, fmt=plistlib.FMT_BINARY)


def records(app_name):
    window = {
        "WindowBounds": f"{{{{{WINDOW_X}, {WINDOW_Y}}}, {{{WINDOW_W}, {WINDOW_H}}}}}",
        "ShowSidebar": False,
        "ShowToolbar": False,
        "ShowPathbar": False,
        "ShowStatusBar": False,
        "ShowTabView": False,
        "ContainerShowSidebar": False,
        "PreviewPaneVisibility": False,
        "SidebarWidth": 180,
    }
    # `backgroundType` 0 leaves Finder's own window background in place, which
    # follows the reader's appearance. A solid colour here would be a white
    # panel in dark mode.
    icons = {
        "viewOptionsVersion": 1,
        "backgroundType": 0,
        "gridOffsetX": 0.0,
        "gridOffsetY": 0.0,
        "gridSpacing": 100.0,
        "arrangeBy": "none",
        "iconSize": float(ICON_SIZE),
        "textSize": float(TEXT_SIZE),
        "labelOnBottom": True,
        "showIconPreview": False,
        "showItemInfo": False,
    }

    app_x = WINDOW_W // 2 - ICON_GAP // 2
    link_x = WINDOW_W // 2 + ICON_GAP // 2

    entries = [
        (".", b"bwsp", b"blob", plist(window)),
        (".", b"icvl", b"type", b"icnv"),
        (".", b"icvp", b"blob", plist(icons)),
        (".", b"vSrn", b"long", 1),
        (app_name, b"Iloc", b"blob", iloc(app_x, ICON_Y)),
        ("Applications", b"Iloc", b"blob", iloc(link_x, ICON_Y)),
    ]
    # Finder walks the leaf in order and stops early, so the sort is not
    # cosmetic: by name folded to lower case, then by record code.
    entries.sort(key=lambda entry: (entry[0].lower(), entry[1]))
    return [record(*entry) for entry in entries]


def build(app_name):
    leaf = records(app_name)

    # A leaf node: no child pointer, then the records in order.
    node = struct.pack(b">II", 0, len(leaf)) + b"".join(leaf)
    if len(node) > PAGE_SIZE:
        raise ValueError("the records no longer fit one page; the tree would need splitting")

    # Root node, levels, records, nodes, page size. One leaf standing alone is
    # level zero and the only node, which is what the tree looks like here.
    superblock = struct.pack(b">IIIII", 2, 0, len(leaf), 1, PAGE_SIZE)

    offsets = [address(ROOT_BLOCK), address(DSDB_BLOCK), address(NODE_BLOCK)]
    bookkeeping = b"".join(
        [
            struct.pack(b">II", len(offsets), 0),
            struct.pack(f">{len(offsets)}I", *offsets),
            # The block table is padded to a multiple of 256 entries.
            b"\0" * 4 * (256 - len(offsets)),
            # One name, pointing at the superblock: the B-tree is found by it.
            struct.pack(b">I", 1),
            bytes([len(b"DSDB")]) + b"DSDB" + struct.pack(b">I", 1),
            # Thirty-two free lists, one per power of two, all empty.
            struct.pack(b">I", 0) * 32,
        ]
    )
    if len(bookkeeping) > ROOT_BLOCK[1]:
        raise ValueError("the bookkeeping block overflowed its own size")

    header = struct.pack(
        b">I4sIII16s",
        1,
        b"Bud1",
        ROOT_BLOCK[0],
        ROOT_BLOCK[1],
        ROOT_BLOCK[0],
        HEADER_UNKNOWN,
    )

    out = bytearray(ROOT_BLOCK[0] + ROOT_BLOCK[1] + 4)
    out[0 : len(header)] = header
    for (offset, _), payload in (
        (DSDB_BLOCK, superblock),
        (NODE_BLOCK, node),
        (ROOT_BLOCK, bookkeeping),
    ):
        out[offset + 4 : offset + 4 + len(payload)] = payload
    return bytes(out)


def main(argv):
    if len(argv) != 3:
        print(f"usage: {argv[0]} <folder> <app-bundle-name>", file=sys.stderr)
        return 2
    folder, app_name = argv[1], argv[2]
    with open(f"{folder}/.DS_Store", "wb") as handle:
        handle.write(build(app_name))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
