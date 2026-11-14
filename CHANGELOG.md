# Changelog

## [0.4.0](https://github.com/muleyuck/doctty/compare/v0.3.0...v0.4.0) (2026-11-14)


### Features

* render b, strong, em, i, code and kbd in raw HTML ([#81](https://github.com/muleyuck/doctty/issues/81)) ([285eeeb](https://github.com/muleyuck/doctty/commit/285eeeb93f500d325f5d520083c0aecba73b6712)), closes [#72](https://github.com/muleyuck/doctty/issues/72)
* render only supported raw HTML and show the rest as Markdown source ([#68](https://github.com/muleyuck/doctty/issues/68)) ([aca4dbc](https://github.com/muleyuck/doctty/commit/aca4dbc36b24a8ad5d49b1d84bc985e5542edcdf))


### Bug Fixes

* drop keys queued during a rich-view reload ([#86](https://github.com/muleyuck/doctty/issues/86)) ([c6d4871](https://github.com/muleyuck/doctty/commit/c6d4871b691f02496fac1b8feb5e447dc4e64ddf))
* keep blocks nested in quotes, lists, callouts and footnotes ([#75](https://github.com/muleyuck/doctty/issues/75)) ([2a5f774](https://github.com/muleyuck/doctty/commit/2a5f774076d46c2641f2cf0dd514cb1a52efb6bc)), closes [#69](https://github.com/muleyuck/doctty/issues/69)
* keep full tile resolution outside Herdr panes ([#82](https://github.com/muleyuck/doctty/issues/82)) ([ff47431](https://github.com/muleyuck/doctty/commit/ff474317938a17ed1b36de3485c6870f8c9bc5c6))
* quit the rich view on Ctrl-C ([#88](https://github.com/muleyuck/doctty/issues/88)) ([9076c1a](https://github.com/muleyuck/doctty/commit/9076c1a4cae57f119d98fa33710832dba4273b89))
* render Mermaid with mermaid-cli 12 and show why a render failed ([#92](https://github.com/muleyuck/doctty/issues/92)) ([1059fba](https://github.com/muleyuck/doctty/commit/1059fbabfcbe92e22c10b2f4ffae5c9a4d691a80))
* stop waiting for a WebKit helper that does not reply ([#91](https://github.com/muleyuck/doctty/issues/91)) ([b02cbbf](https://github.com/muleyuck/doctty/commit/b02cbbf5c46d3922129f1810306c1dbe02d5a1be))
* stop writing temporary HTML next to the previewed Markdown ([#84](https://github.com/muleyuck/doctty/issues/84)) ([69cc169](https://github.com/muleyuck/doctty/commit/69cc16946ad16d889b4f3929e1fe29d7eb20f56a)), closes [#80](https://github.com/muleyuck/doctty/issues/80)


### Performance Improvements

* avoid redundant Markdown parsing before launching the rich renderer ([#106](https://github.com/muleyuck/doctty/issues/106)) ([013b300](https://github.com/muleyuck/doctty/commit/013b30088177be15d4628575a6a46f29eca429b9))
* restore raw HTML tags in linear time ([#103](https://github.com/muleyuck/doctty/issues/103)) ([7e68177](https://github.com/muleyuck/doctty/commit/7e68177f5eae27893df27fb343512d9296e233ed))

## [0.3.0](https://github.com/muleyuck/doctty/compare/v0.2.0...v0.3.0) (2026-11-01)


### ⚠ BREAKING CHANGES

* a single g no longer jumps to the top; press gg. The undocumented J and K no longer scroll; use Ctrl-f/Ctrl-b or PageDown/PageUp.

### Features

* use Vim's default keys for paging and jumping to the top ([#61](https://github.com/muleyuck/doctty/issues/61)) ([a5f3fba](https://github.com/muleyuck/doctty/commit/a5f3fbaadc6514cb8f1927737b15c39a692ac60b))


### Bug Fixes

* clear quarantine on the Homebrew cask's own bundle ([#56](https://github.com/muleyuck/doctty/issues/56)) ([e36a2b8](https://github.com/muleyuck/doctty/commit/e36a2b8c8405f558974418e71278e14d30ed1a50))
* keep viewing when Markdown contains U+FFFE or U+FFFF ([#63](https://github.com/muleyuck/doctty/issues/63)) ([a9d9335](https://github.com/muleyuck/doctty/commit/a9d9335f244744a55b32217c9967a10b76f1b098))


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
