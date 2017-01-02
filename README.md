# To Build

## Prerequisites

* Cairo (library and header files)
* atk (library and header files)
* pango (library and header files)
* gdk-pixbuf
* gtk 3 (library and header files)
  + This also satifies the dependencies for gdk

On most distributions (eg Fedora, Ubuntu/Debian, etc) the header files are located in a `\*-devel` or `\*-dev` package. E.g: `cairo-devel` or `libgtk-3-dev`.

## Build and install

The software can be built with:

    cargo build

And then installed (to `~/.cargo/bin`) with:

    cargo install
