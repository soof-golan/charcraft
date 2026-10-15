# CharCraft

Open-source live character animation in pure Rust: webcam face tracking, lip sync, and puppets built from PSD or AI
artwork. CharCraft opens Character Animator `.puppet` files and `.chproj` project folders, so existing puppets and
projects keep working.

Part of a family with [AniCraft](https://github.com/soof-golan/anicraft) (2D animation editor) on the shared
[anicraft-libs](https://github.com/soof-golan/anicraft-libs) engine, built on the open-source
[Craft](https://github.com/storytold) crates.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the crate map and [AGENTS.md](AGENTS.md) for the rules every change
follows.

## Building

```sh
cargo test --workspace
```

Rust 1.99.1 is pinned in `rust-toolchain.toml`. On Linux the camera backend needs `libclang` at build time (its V4L2
bindings are generated with bindgen); `libv4l-dev` is not required.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [BSD-3-Clause](LICENSE-BSD-3-Clause), at your option. Third-party code
and assets are listed in [ATTRIBUTION.md](ATTRIBUTION.md).

This project is not affiliated with or endorsed by Adobe. Adobe and Character Animator are trademarks of Adobe Inc.;
they are named here only to describe file compatibility.
