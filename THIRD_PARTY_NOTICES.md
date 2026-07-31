# Third-party notices and attribution

TennoScope is licensed under GPL-3.0-only. This file identifies external data and prior art; it does not change the project's license.

## WFCD Warframe Items

At runtime the application downloads a pinned revision of `data/json/All.json` from the [WFCD/warframe-items](https://github.com/WFCD/warframe-items) project and stores a validated local cache. The catalog supplies canonical item names, identifiers, categories, mastery metadata, and Prime-component context. It is data consumed at runtime, not vendored application source.

The pinned upstream revision is published under the MIT License (Copyright 2017 Kaptard). Downstream distributors should retain that license and copyright notice when redistributing a catalog copy or a bundle that includes one. The application's cache records the upstream source and retrieval time.

Warframe names, artwork, and game data are associated with Digital Extremes. This project is unofficial and does not claim ownership of them.

## Bundled currency icons

`app/src/assets/platinum.png` and `app/src/assets/ducats.png` are Digital Extremes' own in-game icons for Platinum and Orokin Ducats, retrieved from the Warframe Wiki (`PlatinumLarge.png`, `OrokinDucats.png`) and rescaled. They are bundled rather than fetched because they label which currency a figure is quoted in, and that must not depend on the network.

They are used unaltered in form, for identification of the currency they depict, in a tool that reads prices in those currencies. Digital Extremes retains all rights in them; they are not covered by this project's GPL-3.0-only license and are not the project's to relicense. A distributor that cannot carry third-party game art should replace or remove them — nothing else in the interface depends on them, and every figure they mark is also named in text beside it.

## Acquisition research and prior art

The Linux and Windows acquisition implementations were written for this project behind its own bounded process-reader and decoder interfaces. The following projects informed the feasibility study and algorithm design:

- [Sikewyrm/FrameForge](https://github.com/Sikewyrm/FrameForge), GPLv3, demonstrated read-only memory scanning, inventory authorization patterns, strict account-snapshot parsing, and a compatible fallback design.
- [Sainan/warframe-api-helper](https://github.com/Sainan/warframe-api-helper) demonstrated the cross-platform authorization/API technique. Its MIT-plus-Commons-Clause terms are not treated as GPL-compatible; this project did not copy its source.
- [WFCD/WFInfo](https://github.com/WFCD/WFInfo) and [soramanew/wfinfo-linux](https://github.com/soramanew/wfinfo-linux) informed research into `EE.log` reward-screen triggers and screenshot/OCR architecture. TennoScope's implementation is independent Rust code: screen capture and the preprocessing pipeline are in-process, and Tesseract is the only external tool.
- AlecaFrame's readable distribution and public documentation informed behavioral comparison only. AlecaFrame is not open-source software, and its code is not included here.

The detailed source review and pinned references are recorded in [`docs/research/warframe-acquisition-existing-implementations.md`](docs/research/warframe-acquisition-existing-implementations.md).

## Bundled Tesseract (Windows installer only)

The Windows installer ships a prebuilt Tesseract OCR binary, the Leptonica library it links against, and the `eng`/`osd` trained data, taken from the [UB Mannheim Tesseract build](https://github.com/UB-Mannheim/tesseract/wiki) and extracted by [`scripts/vendor-windows-tesseract.ps1`](scripts/vendor-windows-tesseract.ps1). Windows has no package manager to depend on, so the alternative is asking every player to install an OCR engine by hand.

[Tesseract](https://github.com/tesseract-ocr/tesseract) is Apache-2.0 (Copyright Google Inc. and contributors) and [Leptonica](http://www.leptonica.org/) is BSD-2-Clause (Copyright Dan Bloomberg). Both are compatible with this project's GPL-3.0-only license; both remain under their own terms and are not relicensed by inclusion. A distributor redistributing the installer must carry those license texts. Linux packages bundle nothing — they take Tesseract from the distribution.

## Rust and JavaScript dependencies

The application is built from third-party crates and npm packages under their respective licenses. `Cargo.lock` and `app/pnpm-lock.yaml` record the resolved dependency graph. The platform-facing ones are worth naming: [xcap](https://github.com/nashaofu/xcap) (Apache-2.0) for screen capture, [image](https://github.com/image-rs/image) (MIT) for the crop pipeline, and [proc-maps](https://github.com/benfred/proc-maps) (MIT), [read-process-memory](https://github.com/luser/read-process-memory) (MIT/Apache-2.0) and [sysinfo](https://github.com/GuillaumeGomez/sysinfo) (MIT) for Windows process access. Each encapsulates its own platform `unsafe`, which is what lets every crate in this workspace keep `#![forbid(unsafe_code)]`. Binary distributors are responsible for producing any dependency-license bundle required by their distribution.
