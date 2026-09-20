# globe

Customizable ASCII globe generator library.

One glyph per terminal character cell, flat in memory. The `Glyph` alphabet
picks how many surface samples each cell carries: `Braille` (2x4 dots, the
default), `Half` (upper/lower half blocks) or `Ascii` (one palette glyph).

## Credits

Rendering math based on 
[C++ code by DinoZ1729](https://github.com/DinoZ1729/Earth).
