param(
    [Parameter(Mandatory)] [string] $BundlePath,
    [Parameter(Mandatory)] [string] $Version,
    [switch] $VerifyUpgrade
)

$ErrorActionPreference = 'Stop'
if (!$env:GITHUB_ACTIONS -or !$env:RUNNER_TEMP) {
    throw 'Native font flow requires a disposable Windows CI runner'
}

$installer = @(Get-ChildItem -LiteralPath $BundlePath -Recurse -File -Filter "FontFerry_${Version}_x64-setup.exe")
if ($installer.Count -ne 1) { throw 'Expected one candidate installer' }
$stage = Join-Path $env:PUBLIC ('fontferry-acceptance-' + [guid]::NewGuid())
$null = New-Item -ItemType Directory -Path $stage
Copy-Item -LiteralPath $installer[0].FullName -Destination (Join-Path $stage 'candidate.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'windows-font-flow.mjs') -Destination $stage
$node = (Get-Command node.exe -ErrorAction Stop).Source
$baseline = $null
if ($VerifyUpgrade) {
    $baseline = Join-Path $stage 'alpha7-setup.exe'
    Invoke-WebRequest 'https://github.com/teckc/fontferry/releases/download/v0.2.0-alpha.7/FontFerry_0.2.0-alpha.7_x64-setup.exe' -OutFile $baseline
    if ((Get-FileHash -LiteralPath $baseline).Hash -ne 'b6d6109c2c93ace139fd45233ded8a28ffe8640e614404a4fedcb8b49bfabcd7') {
        throw 'Unexpected official alpha.7 installer digest'
    }
}
$username = 'ff-' + (Get-Random -Minimum 100000 -Maximum 999999)
$password = ConvertTo-SecureString ([guid]::NewGuid().ToString() + '!Aa1') -AsPlainText -Force
$credential = [pscredential]::new("$env:COMPUTERNAME\$username", $password)
$null = New-LocalUser -Name $username -Password $password -Description 'Disposable FontFerry acceptance account'
try {
    $principal = "$env:COMPUTERNAME\$username"
    icacls.exe $stage /grant "${principal}:(OI)(CI)RX" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not grant test account access' }
    $script = Join-Path $stage 'windows-font-flow.mjs'
    $candidate = Join-Path $stage 'candidate.exe'
    $stdout = Join-Path $env:RUNNER_TEMP 'fontferry-native-stdout.txt'
    $stderr = Join-Path $env:RUNNER_TEMP 'fontferry-native-stderr.txt'
    $arguments = "`"$script`" `"$candidate`" $username $Version"
    if ($baseline) { $arguments += " `"$baseline`"" }
    $process = Start-Process -FilePath $node -ArgumentList $arguments `
        -Credential $credential -LoadUserProfile -WorkingDirectory $stage `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit(720000)) {
        Stop-Process -Id $process.Id -Force
        throw 'Native font flow timed out'
    }
    Get-Content -LiteralPath $stdout
    Get-Content -LiteralPath $stderr
    if ($process.ExitCode -ne 0) { throw "Native font flow exited with $($process.ExitCode)" }
} finally {
    Remove-LocalUser -Name $username
}
