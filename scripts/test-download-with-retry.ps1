$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'download-with-retry.ps1')

$temporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "tennoscope-download-retry-$([guid]::NewGuid())"
$output = Join-Path $temporaryDirectory 'artifact.bin'
New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

try {
  $script:attempts = 0

  function Invoke-WebRequest {
    param(
      [Parameter(Mandatory)] [string] $Uri,
      [Parameter(Mandatory)] [string] $OutFile
    )

    $script:attempts++
    if ($script:attempts -lt 3) {
      [System.IO.File]::WriteAllText($OutFile, 'partial')
      throw [System.Net.WebException]::new('transient download failure')
    }

    [System.IO.File]::WriteAllText($OutFile, 'complete artifact')
  }

  Invoke-DownloadWithRetry `
    -Uri 'https://example.invalid/artifact.bin' `
    -OutFile $output `
    -MaxAttempts 3 `
    -RetryDelaySeconds 0

  if ($script:attempts -ne 3) {
    throw "expected 3 download attempts, observed $script:attempts"
  }

  $content = [System.IO.File]::ReadAllText($output)
  if ($content -ne 'complete artifact') {
    throw "expected the successful artifact, observed '$content'"
  }
} finally {
  Remove-Item -Recurse -Force $temporaryDirectory -ErrorAction SilentlyContinue
}
