# Builds the release exes and packages them as target\installer\ArtCraftAppsManager-<version>-x64.msi,
# for 64-bit Windows, and ArtCraftAppsManager-<version>-x86.msi, for 32-bit Windows.
# -Arch x64 or -Arch x86 builds just one.
#
# Needs the WiX v5 .NET tool and its Util and UI extensions, and Rust's 32-bit
# target (one-time setup):
#   dotnet tool install --global wix --version 5.0.2
#   wix extension add -g WixToolset.Util.wixext/5.0.2
#   wix extension add -g WixToolset.UI.wixext/5.0.2
#   rustup target add i686-pc-windows-msvc
#
# Failures are detected from exit codes rather than $ErrorActionPreference,
# because Windows PowerShell treats cargo's progress output on stderr as errors.
param([ValidateSet('x64', 'x86')][string[]]$Arch = @('x64', 'x86'))

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value

    foreach ($a in $Arch) {
        if ($a -eq 'x86') {
            cargo build --release --target i686-pc-windows-msvc
            $exe = "target\i686-pc-windows-msvc\release\artcraft-apps-manager.exe"
        }
        else {
            cargo build --release
            $exe = "target\release\artcraft-apps-manager.exe"
        }
        if ($LASTEXITCODE) { throw "cargo build failed ($a)" }

        $out = "target\installer\ArtCraftAppsManager-$version-$a.msi"
        wix build installer\ArtCraftAppsManager.wxs -arch $a -ext WixToolset.Util.wixext -ext WixToolset.UI.wixext `
            -d "Version=$version" `
            -d "ExePath=$exe" `
            -d "IconPath=assets\logo.ico" `
            -o $out
        if ($LASTEXITCODE) { throw "wix build failed ($a)" }
        Write-Host "Built $out"
    }
}
finally {
    Pop-Location
}
