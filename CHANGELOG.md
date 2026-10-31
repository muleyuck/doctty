# Changelog

## [0.2.1](https://github.com/muleyuck/doctty/compare/v0.2.0...v0.2.1) (2026-10-31)


### Bug Fixes

* clear quarantine on the Homebrew cask's own bundle ([#56](https://github.com/muleyuck/doctty/issues/56)) ([e36a2b8](https://github.com/muleyuck/doctty/commit/e36a2b8c8405f558974418e71278e14d30ed1a50))


### Performance Improvements

* make the low-quality preview match the WebKit tile and stop wasted captures ([#57](https://github.com/muleyuck/doctty/issues/57)) ([bef231b](https://github.com/muleyuck/doctty/commit/bef231b32cbb190b6f6a8fcfe13c4596e47aa6b2))

## [0.2.0](https://github.com/muleyuck/doctty/compare/v0.1.1...v0.2.0) (2026-10-31)


### Features

* detect Kitty graphics support by querying the terminal ([#38](https://github.com/muleyuck/doctty/issues/38)) ([e911849](https://github.com/muleyuck/doctty/commit/e91184975b2138659790cdaf4909c858e13d3215))
* **macos:** integrate precompiled low-CPU Eco renderer ([06f260b](https://github.com/muleyuck/doctty/commit/06f260b998fd2425577ecc191fdcfabba741aaa6))
* **release:** stage verified macOS arm64 ZIP and Cask packaging ([23b5671](https://github.com/muleyuck/doctty/commit/23b567138550598df1e585b927f8319c53883035))
* terminal Markdown viewer for Ghostty/Kitty ([5928197](https://github.com/muleyuck/doctty/commit/5928197bc7e4fe31a0555bf28445c5f10cd7b21a))


### Bug Fixes

* address review follow-ups ([1add054](https://github.com/muleyuck/doctty/commit/1add054a8f619b5205834133d17aa232461868b3))
* describe the TTY requirement in the renderer's non-terminal error ([#46](https://github.com/muleyuck/doctty/issues/46)) ([dfa2588](https://github.com/muleyuck/doctty/commit/dfa258866903b6bf8f73be6b808844ebe8ef5923))
* keep renderer images within Herdr pane limits on wide panes ([#45](https://github.com/muleyuck/doctty/issues/45)) ([a44adf3](https://github.com/muleyuck/doctty/commit/a44adf3a75e89256c286d7773ccb18a528cf9776))
* keep renderer tiles within Herdr's Kitty image limits ([#40](https://github.com/muleyuck/doctty/issues/40)) ([bb13ac8](https://github.com/muleyuck/doctty/commit/bb13ac83727cc7baa90b60c3bdecda777e6149ac))
* **macos:** diagnose possible Gatekeeper block of Swift helper ([278f7a0](https://github.com/muleyuck/doctty/commit/278f7a038e6da00b36ba2feb417aec8c084c1042))
* remove committed node_modules and ignore it ([6addedd](https://github.com/muleyuck/doctty/commit/6addeddf57cb503c43aece9c9c23c56ddffb2e6b))
* remove the renderer's HTML lease when the terminal is closed ([#52](https://github.com/muleyuck/doctty/issues/52)) ([868e2bb](https://github.com/muleyuck/doctty/commit/868e2bbe2c3fdf49ea6752b915a8a6cfba71c25c))
* unify macOS installs on the standard compiled renderer ([f85b3c3](https://github.com/muleyuck/doctty/commit/f85b3c39a9f01cd12a722f6417a9905981057818))


### Performance Improvements

* send renderer tiles as zlib-compressed RGB instead of PNG ([#51](https://github.com/muleyuck/doctty/issues/51)) ([a95347a](https://github.com/muleyuck/doctty/commit/a95347a2bdebc9b63f96f536967dce746687cf65))
* show high-quality renderer tiles while scrolling ([#43](https://github.com/muleyuck/doctty/issues/43)) ([d1b9853](https://github.com/muleyuck/doctty/commit/d1b985305e7ec78ba1ebf0abfbf0f09785c8ada4))
