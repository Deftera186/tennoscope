function Invoke-DownloadWithRetry {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory)] [string] $Uri,
    [Parameter(Mandatory)] [string] $OutFile,
    [ValidateRange(1, 100)] [int] $MaxAttempts = 3,
    [ValidateRange(0, 3600)] [int] $RetryDelaySeconds = 5
  )

  for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
    try {
      Invoke-WebRequest -Uri $Uri -OutFile $OutFile
      return
    } catch {
      Remove-Item -Force $OutFile -ErrorAction SilentlyContinue

      if ($attempt -eq $MaxAttempts) {
        throw
      }

      Write-Warning "Download attempt $attempt of $MaxAttempts failed: $($_.Exception.Message)"
      Start-Sleep -Seconds $RetryDelaySeconds
    }
  }
}
