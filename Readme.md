# WiitarMap

Wii Guitar remapping using `evsieve`

## Requirements

This is built with SteamOS as its primary consumer. Specifics will differ per distro. I develop in an Arch Linux container using [Distrobox](https://distrobox.it).

- Rust
- `libudev` (in `core/systemd-libs` on Arch Linux)
- [`evsieve`](https://github.com/KarsMulder/evsieve)
  - `libevdev` (in `libevdev` on Arch Linux)
- `pkg-config` (in `pkgconf` on Arch Linux)

### Arch Linux one-liner

```bash
sudo pacman -S rust systemd-libs libevdev pkgconf
```

## Setup

Something like this:
