# Vendor a portable Tesseract for the Windows bundle.
#
# Windows has no package manager to depend on, and "install Tesseract first" is exactly the extra
# step the installer exists to remove -- so the engine ships inside it. The UB-Mannheim build is
# upstream's only maintained Windows binary; its installer is NSIS, so 7-Zip extracts the payload
# without running it. Tesseract is Apache-2.0 and Leptonica is BSD-2-Clause, both compatible with
# this project's GPL-3.0-only, and both are credited in THIRD_PARTY_NOTICES.md.
#
# Only what the OCR path actually calls is kept: the executable, the DLLs beside it, and English
# training data. The full install is ~600 MB of language packs the reward reader never opens.
$ErrorActionPreference = 'Stop'

$version = '5.5.0.20241111'
$url = "https://digi.bib.uni-mannheim.de/tesseract/tesseract-ocr-w64-setup-$version.exe"
$root = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $root 'app/src-tauri/vendor/tesseract'
$staging = Join-Path $env:RUNNER_TEMP 'tesseract-extract'

if (Test-Path (Join-Path $vendor 'tesseract.exe')) {
  Write-Host 'tesseract already vendored'
  exit 0
}

$installer = Join-Path $env:RUNNER_TEMP 'tesseract-setup.exe'
Invoke-WebRequest -Uri $url -OutFile $installer
Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
& 7z x $installer "-o$staging" | Out-Null

New-Item -ItemType Directory -Force -Path $vendor | Out-Null
Copy-Item (Join-Path $staging 'tesseract.exe') $vendor
Copy-Item (Join-Path $staging '*.dll') $vendor
Copy-Item (Join-Path $staging 'tessdata/eng.traineddata') $vendor
Copy-Item (Join-Path $staging 'tessdata/osd.traineddata') $vendor

# A missing DLL turns every reward read into "tesseract is not available" at the one moment the
# player is watching, so the bundle is proven to run here rather than on their machine.
$probe = & (Join-Path $vendor 'tesseract.exe') --tessdata-dir $vendor --version
if ($LASTEXITCODE -ne 0) { throw 'the vendored tesseract does not run' }
Write-Host $probe
