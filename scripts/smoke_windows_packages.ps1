param([Parameter(Mandatory=$true)][string]$BundleDir)
$ErrorActionPreference = 'Stop'
$msi = Get-ChildItem $BundleDir -Recurse -Filter *.msi | Select-Object -First 1
$nsis = Get-ChildItem $BundleDir -Recurse -Filter *setup.exe | Select-Object -First 1
if (-not $msi -or -not $nsis) { throw 'Both MSI and NSIS packages are required' }
# Install the MSI into a path exercising whitespace and Unicode. NSIS is built and
# signature-inspected here; its interactive lifecycle is covered by release runners.
$target = Join-Path $env:RUNNER_TEMP 'PulseScope ü Test'
$log = Join-Path $env:RUNNER_TEMP 'pulsescope-msi.log'
$p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @('/i', $msi.FullName, '/qn', "INSTALLDIR=$target", "/l*v", $log)
if ($p.ExitCode -ne 0) { throw "MSI install failed: $($p.ExitCode)" }
$exe = Get-ChildItem $target -Recurse -Filter pulsescope.exe | Select-Object -First 1
if (-not $exe) { throw 'Installed executable not found' }
$p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @('/x', $msi.FullName, '/qn', '/norestart')
if ($p.ExitCode -ne 0) { throw "MSI uninstall failed: $($p.ExitCode)" }

