# Installation

## Macos

`brew install aravis`

# Windows mingw
```
install https://www.msys2.org/
pacman -S mingw-w64-x86_64-aravis-gst mingw-w64-x86_64-pkg-config
rustup toolchain add stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu run
```
