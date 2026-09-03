# Binary Inspection

Only declared text symbols are disassembled. Nothing is scanned for code. In a linked image,
the entry point and the exported functions are symbols too, one symbol per address.
The functions a linked image's unwind table states, an x86-64 PE's `.pdata` or an ELF's
`.eh_frame`, are symbols too; one nothing else names is called `<function 0x…>` by its
address, with the stated length as its size. A relocatable object's `.eh_frame` is not read.
A PE's chained unwind entry, a function fragment, is a symbol of its own, `<fragment 0x…>`,
and its parent's extent stops where it begins.

Line info comes from the binary's debug info.
A function's extent is the smaller of what the debug info gives and the distance to the next
symbol. An unwind entry covering the function states its end instead.

Code is decoded as the architecture the object declares; x32 is decoded as 64-bit code.

Inspection runs off the UI thread. A newer request replaces one not yet started, and an
answer for a symbol no longer shown is dropped.
Demangling runs on several threads, with the same result in the same order every time.

No file input can make the app panic. A broken file loads as far as it can be read.

## PDB

A PE names its `.pdb`, which is looked for at the recorded path and beside the binary, and used
only when its GUID and age match the image. It gives the source view its line info, and each
source file's checksum, so an edited file is told from the one compiled. The functions its
modules record are symbols too, with their lengths, after what the image itself declares; so
are its public symbols for code, demangled, with size 0.

## Implementation notes

Disassembly is behind one seam, and the architecture-specific code is dispatched statically,
not through a trait object, for performance: ideally the architecture is decided once when
the object is loaded, and nothing per instruction is dynamically dispatched.
