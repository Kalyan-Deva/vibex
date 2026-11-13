# Third-party notices

Vibex's own code is MIT OR Apache-2.0 (see [README.md](README.md#license)). The installer also ships these separate programs, each under its own licence:

| Program | From | Licence | Where it's installed |
|---|---|---|---|
| OpenConsole (the console host that lets images through) | [microsoft/terminal](https://github.com/microsoft/terminal) | MIT | `OpenConsole.exe`, licence in `OpenConsole-LICENSE.txt` |
| Neovim | [neovim/neovim](https://github.com/neovim/neovim), the official Windows build | Apache-2.0 and the Vim licence | `tools\nvim`, licence in `tools\LICENSES\neovim.txt` |
| GNU nano | [nano-editor.org](https://www.nano-editor.org), as built by [MSYS2](https://www.msys2.org) | GPL-3.0-or-later | `tools\nano` |
| nano's runtime: msys2-runtime, ncurses, libintl, libiconv, file (libmagic), gcc-libs, zlib, bzip2, xz, zstd | [MSYS2](https://packages.msys2.org) | their own licences (GPL, LGPL, MIT, BSD and others) | `tools\nano\usr\bin`, every licence in `tools\LICENSES\msys2-*` |

These programs aren't linked into Vibex. They're shipped beside it, and Vibex runs them as separate processes. The exact versions and SHA-256s are in [tools/bundle.lock](tools/bundle.lock).

The source for each MSYS2 package is in the [MSYS2-packages](https://github.com/msys2/MSYS2-packages) repository and at [mirror.msys2.org/msys/sources](https://mirror.msys2.org/msys/sources/). The installed `tools\LICENSES\SOURCES.txt` lists the exact package versions.

The extra colour themes (Catppuccin, Dracula, Nord, Gruvbox, Rosé Pine, Kanagawa, Everforest, GitHub, Tokyo Night variants and others) come from the [iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes) collection (MIT, Copyright Mark Badolato; each scheme is its authors' work), Solarized from Ethan Schoonover's published palette, and Campbell from Windows Terminal ([microsoft/terminal](https://github.com/microsoft/terminal), MIT).

Rust crates that Vibex is built with are listed in `Cargo.lock`. Each crate keeps its own licence, and all of them are permissive (MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0, Unlicense or Unicode-3.0).
