# Builds the release exe and packages it as target\installer\ArtCraftAppsManager-<version>-x64.msi.
#
# Needs the WiX v5 .NET tool and its Util extension (one-time setup):
#   dotnet tool install --global wix --version 5.0.2
#   wix extension add -g WixToolset.Util.wixext/5.0.2
#
# Failures are detected from exit codes rather than $ErrorActionPreference,
# because Windows PowerShell treats cargo's progress output on stderr as errors.
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE) { throw "cargo build failed" }

    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value

    $out = "target\installer\ArtCraftAppsManager-$version-x64.msi"
    wix build installer\ArtCraftAppsManager.wxs -arch x64 -ext WixToolset.Util.wixext `
        -d "Version=$version" `
        -d "ExePath=target\release\artcraft-apps-manager.exe" `
        -d "IconPath=assets\logo.ico" `
        -o $out
    if ($LASTEXITCODE) { throw "wix build failed" }
    Write-Host "Built $out"
}
finally {
    Pop-Location
}
