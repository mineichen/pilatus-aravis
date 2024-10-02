# Installation

## Macos

`brew install aravis`

# Windows mingw
```
winget install --id=MSYS2.MSYS2  -e
# Add to PATH: C:\msys64\mingw64\bin
pacman -S mingw-w64-x86_64-aravis-gst mingw-w64-x86_64-pkg-config
rustup toolchain add stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu run
```
