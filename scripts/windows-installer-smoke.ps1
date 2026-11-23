param(
    [Parameter(Mandatory)] [string] $BundlePath,
    [Parameter(Mandatory)] [string] $Version
)

$ErrorActionPreference = 'Stop'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\FontFerry'
$previousPath = 'HKCU:\Software\github\FontFerry'
$data = Join-Path $env:LOCALAPPDATA 'teckc/FontFerry/data'
$db = Join-Path $data fontferry.db
$journal = Join-Path $data font-operation.json
$default = Join-Path $env:LOCALAPPDATA FontFerry
$custom = Join-Path $env:RUNNER_TEMP 'FontFerry Upgrade Test'

function Invoke-SmokeProcess([string] $Path, [string] $Arguments, [int] $Expected = 0) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit(120000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Process timed out: $Path"
    }
    if ($process.ExitCode -ne $Expected) {
        throw "$Path returned $($process.ExitCode), expected $Expected"
    }
}

function Assert-Candidate([string] $Location) {
    $program = Join-Path $Location fontferry.exe
    if ((Get-ItemPropertyValue $key DisplayVersion) -ne $Version) { throw 'Wrong installed version' }
    if ((Get-FileHash -LiteralPath $program).Hash -ne $expectedBinary) {
        throw 'Installed executable differs from the installer payload'
    }
    Invoke-SmokeProcess $program doctor
    if (!(Test-Path -LiteralPath $db)) { throw 'Missing initialized database' }
}

function Uninstall-Candidate([string] $Location) {
    Invoke-SmokeProcess (Join-Path $Location uninstall.exe) '/S'
    for ($attempt = 0; $attempt -lt 60 -and (Test-Path -LiteralPath $key); $attempt++) {
        Start-Sleep -Milliseconds 500
    }
    if ((Test-Path -LiteralPath $key) -or (Test-Path -LiteralPath (Join-Path $Location fontferry.exe))) {
        throw 'Application uninstall incomplete'
    }
}

if (!$env:GITHUB_ACTIONS -or !$env:RUNNER_TEMP -or
    (Test-Path -LiteralPath $key) -or (Test-Path -LiteralPath $data)) {
    throw 'Installer smoke test requires a clean GitHub Actions runner'
}
$installers = @(Get-ChildItem $BundlePath -Recurse -File -Filter "FontFerry_$($Version)_x64-setup.exe")
if ($installers.Count -ne 1) { throw 'Expected one Windows installer matching the release version' }
$installer = $installers[0].FullName
$payload = Join-Path $env:RUNNER_TEMP fontferry-payload
& (Join-Path $env:ProgramFiles '7-Zip/7z.exe') x -y "-o$payload" $installer
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract NSIS payload' }
$programs = @(Get-ChildItem $payload -Recurse -File -Filter fontferry.exe)
if ($programs.Count -ne 1) { throw 'Expected one fontferry.exe in NSIS bundle' }
$expectedBinary = (Get-FileHash -LiteralPath $programs[0].FullName).Hash

Invoke-SmokeProcess $installer '/S'
Assert-Candidate $default
$app = Start-Process -FilePath (Join-Path $default fontferry.exe) -PassThru
try {
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        $app.Refresh()
        if ($app.HasExited) { throw "Desktop startup exited with $($app.ExitCode)" }
        if ($app.MainWindowTitle -eq 'FontFerry') { break }
        Start-Sleep -Milliseconds 500
    }
    if ($app.MainWindowTitle -ne 'FontFerry') { throw 'Desktop window never appeared' }
} finally {
    if (!$app.HasExited) {
        if (!$app.CloseMainWindow() -or !$app.WaitForExit(10000)) {
            Stop-Process -Id $app.Id -Force
        }
    }
}
Uninstall-Candidate $default
if (!(Test-Path -LiteralPath $db)) { throw 'Application uninstall removed management data' }
Write-Host 'PASS: fresh install, desktop and CLI startup, uninstall with retained ledger'

$baseline = Join-Path $env:RUNNER_TEMP alpha6-setup.exe
Invoke-WebRequest 'https://github.com/teckc/fontferry/releases/download/v0.2.0-alpha.6/FontFerry_0.2.0-alpha.6_x64-setup.exe' -OutFile $baseline
if ((Get-FileHash -LiteralPath $baseline).Hash -ne 'b52f6946a8365f6420f1eaa3877a847f7b4798b357b8b7af679354e5a66c1414') {
    throw 'Unexpected official alpha.6 installer digest'
}
Invoke-SmokeProcess $baseline "/S /D=$custom"
if ((Get-ItemPropertyValue $key DisplayVersion) -ne '0.2.0-alpha.6' -or
    (Get-Item $previousPath).GetValue('') -ne $custom) {
    throw 'Baseline install did not register its custom path'
}
Invoke-SmokeProcess (Join-Path $custom fontferry.exe) doctor
$env:FF_TEST_DB = $db
python -c 'import os, sqlite3; db=sqlite3.connect(os.environ["FF_TEST_DB"]); db.execute("INSERT OR REPLACE INTO settings (key, value_json) VALUES (?, ?)", ("update-channel", chr(34)+"beta"+chr(34))); db.commit(); db.close()'
if ($LASTEXITCODE -ne 0) { throw 'Could not seed update-channel setting' }
$backup = Join-Path $data 'backups/ci-sentinel.txt'
'retained backup' | Set-Content -LiteralPath $backup
$backupHash = (Get-FileHash -LiteralPath $backup).Hash

Invoke-SmokeProcess $installer '/S'
Assert-Candidate $custom
if ((Get-Item $previousPath).GetValue('') -ne $custom -or
    (Test-Path -LiteralPath (Join-Path $default fontferry.exe))) {
    throw 'Upgrade did not reuse the custom location'
}
python -c 'import os, sqlite3; db=sqlite3.connect(os.environ["FF_TEST_DB"]); assert db.execute("SELECT value_json FROM settings WHERE key=?", ("update-channel",)).fetchone() == (chr(34)+"beta"+chr(34),); db.close()'
if ($LASTEXITCODE -ne 0) { throw 'Update-channel setting did not survive upgrade' }
if ((Get-FileHash -LiteralPath $backup).Hash -ne $backupHash) { throw 'Upgrade modified a backup' }
Write-Host 'PASS: alpha.6 custom-path upgrade, settings and backup retention'

Move-Item -LiteralPath $db -Destination "$db.saved"
'{}' | Set-Content -LiteralPath $journal
$journalHash = (Get-FileHash -LiteralPath $journal).Hash
Invoke-SmokeProcess (Join-Path $custom fontferry.exe) doctor 1
if ((Test-Path -LiteralPath $db) -or (Get-FileHash -LiteralPath $journal).Hash -ne $journalHash) {
    throw 'Recovery guard modified evidence for a missing ledger'
}
Uninstall-Candidate $custom
if ((Get-FileHash -LiteralPath $journal).Hash -ne $journalHash -or
    (Get-FileHash -LiteralPath $backup).Hash -ne $backupHash -or !(Test-Path -LiteralPath "$db.saved")) {
    throw 'Uninstall modified recovery evidence'
}
Write-Host 'PASS: missing-ledger recovery guard and recovery evidence retention'
