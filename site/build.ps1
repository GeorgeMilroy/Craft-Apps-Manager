# Builds the landing page into site\public: the pages in site\src, the app's
# logo, icons and fonts, and the newest installers (64-bit and 32-bit). Run
# installer\build-installer.ps1 first.
#
#   site\build.ps1           build only (preview with: firebase serve, in site\)
#   site\build.ps1 -Deploy   build, then publish to every site in hosting.json
#
# Deploying uses the account the Firebase CLI is signed in with, or with
# -KeyFile <path>, a service account key for the project.
param([switch]$Deploy, [string]$KeyFile)

$site = $PSScriptRoot
$root = Split-Path -Parent $site
$public = Join-Path $site 'public'
$utf8 = New-Object System.Text.UTF8Encoding $false

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
$x64, $x86 = foreach ($arch in 'x64', 'x86') {
    $file = "ArtCraftAppsManager-$version-$arch.msi"
    $msi = Join-Path $root "target\installer\$file"
    if (-not (Test-Path $msi)) { throw "$msi is missing. Run installer\build-installer.ps1 first." }
    $info = Get-Item $msi
    [pscustomobject]@{
        File   = $file
        Path   = $msi
        Length = $info.Length
        Size   = '{0:0.0} MB' -f ($info.Length / 1MB)
        Sha256 = (Get-FileHash $msi -Algorithm SHA256).Hash.ToLowerInvariant()
        Time   = $info.LastWriteTime
    }
}

if (Test-Path $public) { Remove-Item $public -Recurse -Force }
Copy-Item "$site\src" $public -Recurse
New-Item -ItemType Directory -Force "$public\fonts", "$public\assets\apps", "$public\download" | Out-Null
Copy-Item "$root\assets\fonts\*" "$public\fonts"
Copy-Item "$root\assets\app-icons\*.png" "$public\assets\apps"
Copy-Item "$root\assets\logo.png" "$public\assets\logo.png"
Copy-Item "$root\assets\logo.ico" "$public\favicon.ico"
Copy-Item $x64.Path, $x86.Path "$public\download"

# The 64-bit installer is the main download; the _x86 values are for 32-bit Windows.
$released = $x64.Time
$values = @{
    version    = $version
    file       = $x64.File
    size       = $x64.Size
    sha256     = $x64.Sha256
    file_x86   = $x86.File
    size_x86   = $x86.Size
    sha256_x86 = $x86.Sha256
    released   = $released.ToString('d MMMM yyyy', [Globalization.CultureInfo]::InvariantCulture)
}
foreach ($page in Get-ChildItem $public -Filter *.html) {
    $html = [IO.File]::ReadAllText($page.FullName)
    foreach ($key in $values.Keys) { $html = $html.Replace("{{$key}}", $values[$key]) }
    [IO.File]::WriteAllText($page.FullName, $html, $utf8)
}

# For anything that wants the current version, such as a future self-updater.
$latest = [ordered]@{
    version  = $version
    url      = "/download/$($x64.File)"
    size     = $x64.Length
    sha256   = $x64.Sha256
    released = $released.ToString('yyyy-MM-dd')
    x86      = [ordered]@{
        url    = "/download/$($x86.File)"
        size   = $x86.Length
        sha256 = $x86.Sha256
    }
}
[IO.File]::WriteAllText("$public\latest.json", ($latest | ConvertTo-Json), $utf8)

# firebase.json: the shared Hosting config from hosting.json, once per site
# (Firebase allows only one site per hosting entry), with /download and
# /download/x86 pointing at this version.
$hosting = [IO.File]::ReadAllText("$site\hosting.json").Replace('{{file}}', $x64.File).Replace('{{file_x86}}', $x86.File) | ConvertFrom-Json
$entries = foreach ($name in $hosting.sites) {
    $entry = [ordered]@{ site = $name }
    foreach ($p in $hosting.config.PSObject.Properties) { $entry[$p.Name] = $p.Value }
    $entry
}
[IO.File]::WriteAllText("$site\firebase.json", (@{ hosting = @($entries) } | ConvertTo-Json -Depth 10), $utf8)

Write-Host "Built $public for version $version"

if ($Deploy) {
    $saved = $env:GOOGLE_APPLICATION_CREDENTIALS, $env:XDG_CONFIG_HOME
    Push-Location $site
    try {
        if ($KeyFile) {
            # The CLI prefers any signed-in account over a key, so give it an
            # empty settings folder to make it use the key.
            $env:GOOGLE_APPLICATION_CREDENTIALS = (Resolve-Path $KeyFile).Path
            $env:XDG_CONFIG_HOME = Join-Path ([IO.Path]::GetTempPath()) 'artcraft-site-deploy'
            New-Item -ItemType Directory -Force $env:XDG_CONFIG_HOME | Out-Null
        }
        firebase deploy --only hosting --non-interactive
        if ($LASTEXITCODE) { throw "firebase deploy failed" }
    }
    finally {
        $env:GOOGLE_APPLICATION_CREDENTIALS, $env:XDG_CONFIG_HOME = $saved
        Pop-Location
    }
}
