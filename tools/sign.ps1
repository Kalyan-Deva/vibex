# Authenticode-sign files with a code-signing certificate from your store.
#
#   powershell -ExecutionPolicy Bypass -File tools\sign.ps1 -Thumbprint <SHA1> -Files a.exe,b.exe
#
# Needs signtool.exe (Windows SDK) and a code-signing certificate (an OV/EV
# cert from a CA, or Azure Trusted Signing). Without one, SmartScreen warns on
# first run of the installer.
param(
    [Parameter(Mandatory = $true)][string]$Thumbprint,
    [Parameter(Mandatory = $true)][string[]]$Files,
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)
$ErrorActionPreference = 'Stop'

$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $signtool) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $signtool = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $signtool) { throw "signtool.exe not found: install the Windows SDK (Signing Tools)" }

foreach ($f in $Files) {
    if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { throw "no such file: $f" }
    & $signtool sign /sha1 $Thumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 /d 'Vibex Terminal' $f
    if ($LASTEXITCODE) { throw "signing $f failed" }
    & $signtool verify /pa $f | Out-Null
    if ($LASTEXITCODE) { throw "signature on $f doesn't verify" }
    Write-Output "signed $f"
}
