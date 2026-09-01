#!/usr/bin/env python3
"""Write the `.DS_Store` that gives the mounted image its window.

Finder keeps a folder's window frame, view style, icon positions, and
background picture in a `.DS_Store` inside that folder and nowhere else. A
volume without one opens at whatever size and view Finder last used for an
unknown folder, sidebar and toolbar showing, and with nothing in the panel but
two icons: a file browser pointed at a disk, rather than the drag-to-install
window a reader expects, and no sentence anywhere saying what to do with it.

The other way to produce that window is to drive Finder over Apple events, as
`create-dmg` does. It needs a GUI session, and on a CI runner it fails as a TCC
denial or an AppleEvent timeout, so it is not an option here. Nothing below
talks to Finder or to the network, and nothing has to be installed for it: the
interpreter and `plistlib` arrive with the Command Line Tools that build the
binary in the first place.

Only the records that window needs are written, all in one B-tree leaf:

    bwsp   the window frame, with toolbar, sidebar, and pathbar hidden
    icvp   icon view: icon size, text size, the background picture, no arranging
    icvl   the view style, so the volume opens in icon view at all
    vSrn   a version marker Finder expects to find beside them
    Iloc   one per icon: where the app and the `Applications` link sit

This runs against the *mounted* image rather than the folder it was staged
from, and that is what the background picture costs. Finder does not name that
file by path: `icvp` carries a Carbon alias record, which identifies it by
volume name, volume creation date, and catalogue node ID, and none of those
exist until the volume does. `packaging/build-dmg.sh` therefore builds a
read-write image, attaches it, runs this, detaches, and converts to the
compressed image that ships. The alias also carries the file's path twice, once
absolute and once as the mount point it hangs off, so a resolver that finds the
volume mounted somewhere else — `/Volumes/hww 1`, because the reader already
had an hww image open — still lands on the picture.

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

import os
import plistlib
import struct
import sys

# The window, in Finder's coordinates: a panel wide enough for two 128px icons
# with the arrow between them, and tall enough for two lines of text under
# them. The background picture is drawn into the content area at its natural
# size, so `assets/dmg/make-background.py` draws exactly this many pixels and
# the two files move together.
WINDOW_X, WINDOW_Y = 400, 180
WINDOW_W, WINDOW_H = 600, 400

ICON_SIZE = 128
TEXT_SIZE = 12

# Icon centres, derived from the window rather than written out, so moving the
# window keeps the pair centred in it. The gap is centre to centre.
ICON_GAP = 270
ICON_Y = 168

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

# Seconds between the Mac epoch of 1904-01-01 and the Unix one. Every date in
# an alias record is counted from the first.
MAC_EPOCH = 2082844800


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


def utf16_string(text):
    """A counted UTF-16 string, as the alias's unicode tags store one."""
    raw = text.encode("utf-16-be")
    return struct.pack(b">H", len(raw) // 2) + raw


def tagged(code, payload):
    """One of the alias record's variable-length trailing items."""
    block = struct.pack(b">hH", code, len(payload)) + payload
    return block + b"\0" * (len(payload) % 2)


def birth_time(entry):
    """A file's creation date. HFS+ has one; a filesystem without it has none."""
    try:
        return entry.st_birthtime
    except AttributeError as missing:
        raise SystemExit(
            "this filesystem records no creation date, so no alias can name a "
            "file on it; run this against a mounted HFS+ image"
        ) from missing


def alias(mount, volume, relative):
    """A Carbon alias record naming a file on the volume mounted at `mount`.

    Version 2, which is the only version Finder writes into an `icvp` and the
    one every tool that reads a `.DS_Store` expects. The 150-byte fixed part
    identifies the file three redundant ways — by catalogue node ID, by Carbon
    path, and by POSIX path — and a resolver falls back through them in that
    order. Only the first is exact, and it is the one that cannot be known
    before the volume exists, which is why this takes a mount point.

    Inode numbers *are* catalogue node IDs on HFS+. They are not on APFS, where
    they run past 32 bits; a disk image built with a different `-fs` would
    truncate them here and fall through to the paths, which is the reason
    `build-dmg.sh` names HFS+ rather than taking hdiutil's default.
    """
    parts = relative.split("/")
    target = os.path.join(mount, *parts)
    file_entry = os.stat(target)
    parent = os.path.join(mount, *parts[:-1])
    folder_name = parts[-2] if len(parts) > 1 else volume
    parent_entry = os.stat(parent)
    volume_entry = os.stat(mount)

    # The catalogue node ID of every folder between the volume root and the
    # file, in order; one entry here, for `.background`. The root's own ID is
    # not in the list: a resolver starts from the volume and walks down.
    ancestors = []
    walk = mount
    for part in parts[:-1]:
        walk = os.path.join(walk, part)
        ancestors.append(os.stat(walk).st_ino & 0xFFFFFFFF)

    fixed = struct.pack(
        b">h28pI2sHI64pII4s4shhI2s10s",
        0,  # kind: a file, not a folder
        volume.encode("utf-8"),
        (int(birth_time(volume_entry)) + MAC_EPOCH) & 0xFFFFFFFF,
        b"H+",
        5,  # an ejectable disk, which is what a mounted image is
        parent_entry.st_ino & 0xFFFFFFFF,
        parts[-1].encode("utf-8"),
        file_entry.st_ino & 0xFFFFFFFF,
        (int(birth_time(file_entry)) + MAC_EPOCH) & 0xFFFFFFFF,
        b"\0\0\0\0",  # creator code: none, this is not a Carbon application
        b"\0\0\0\0",  # type code, likewise
        -1,  # levels from and to: unknown, which is what every writer says
        -1,
        0,  # volume attributes and filesystem ID, neither of which a
        b"\0\0",  # resolver needs to find a file on a volume it has mounted
        b"\0" * 10,
    )

    trailer = b"".join(
        [
            tagged(0, folder_name.encode("utf-8")),  # the parent folder's name
            tagged(1, struct.pack(f">{len(ancestors)}I", *ancestors)),
            tagged(2, ":".join([volume, *parts]).encode("utf-8")),  # Carbon path
            tagged(14, utf16_string(parts[-1])),
            tagged(15, utf16_string(volume)),
            # The same two dates again at full precision. A resolver that
            # compares the 32-bit ones is comparing a truncation of these.
            tagged(16, struct.pack(b">d", birth_time(volume_entry) + MAC_EPOCH)),
            tagged(17, struct.pack(b">d", birth_time(file_entry) + MAC_EPOCH)),
            tagged(18, target.encode("utf-8")),
            tagged(19, mount.encode("utf-8")),
            struct.pack(b">hH", -1, 0),
        ]
    )

    body = fixed + trailer
    size = 8 + len(body)
    return struct.pack(b">4shh", b"\0\0\0\0", size, 2) + body


def records(app_name, background):
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
    # `backgroundType` 2 is a picture, and the picture is transparent except
    # for the arrow and the two lines of text: Finder composites it over its
    # own window background, which follows the reader's appearance. A painted
    # panel, or `backgroundType` 1 and a colour, would be a white rectangle on
    # a Mac in dark mode. See assets/dmg/make-background.py.
    icons = {
        "viewOptionsVersion": 1,
        "backgroundType": 2,
        "backgroundImageAlias": background,
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


def build(app_name, background):
    leaf = records(app_name, background)

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
    if len(argv) != 5:
        print(
            f"usage: {argv[0]} <mounted-volume> <volume-name> <app-bundle-name> "
            "<background-path-in-volume>",
            file=sys.stderr,
        )
        return 2
    mount, volume, app_name, background = argv[1], argv[2], argv[3], argv[4]
    if not os.path.ismount(mount):
        print(f"{mount} is not a mount point; the alias would name nothing", file=sys.stderr)
        return 1
    with open(f"{mount}/.DS_Store", "wb") as handle:
        handle.write(build(app_name, alias(mount, volume, background)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
