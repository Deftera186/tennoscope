# COPR (Fedora) packaging

Project: [`deftera/tennoscope`](https://copr.fedorainfracloud.org/coprs/deftera/tennoscope/)
(`fedora-43-x86_64`, `fedora-44-x86_64`). COPR signs per-project automatically;
the maintainer handles no keys. Cost: $0.

## Layout

- `tennoscope.spec` — source recipe: locked Cargo workspace + pnpm frontend,
  same shape as `packaging/arch/PKGBUILD` (raw binary install, no Tauri bundle
  step). Bump `Version:` per release and append a `%changelog` entry.

## Submit a build

COPR chroots have no network, so dependencies ride inside the SRPM:

```bash
# Per release, from the repo root:
cargo vendor vendor
(cd app && tar -I 'zstd -12' -cf ~/rpmbuild/SOURCES/tennoscope-node-modules-<ver>.tar.zst node_modules)
tar -I 'zstd -12' -cf ~/rpmbuild/SOURCES/tennoscope-vendor-<ver>.tar.zst vendor
cp packaging/copr/tennoscope.spec ~/rpmbuild/SPECS/
rpmbuild -bs ~/rpmbuild/SPECS/tennoscope.spec
copr-cli build deftera/tennoscope ~/rpmbuild/SRPMS/tennoscope-<ver>-1.src.rpm
```

Users enable it with:

```bash
sudo dnf copr enable deftera/tennoscope
sudo dnf install tennoscope
```

## Notes

- `check()` mirrors the Arch recipe, including the one 16:10 OCR skip that the
  combined upstream traineddata requires (see `packaging/arch.md`).
- Rebuilds per release are manual for now; webhook/Packit automation only when
  the release cadence earns it.
