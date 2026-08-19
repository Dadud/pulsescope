# Functional check for the manual-retune bank follow and operator VFO slots.
# Reproduces the reported defect: with FM Broadcast selected, tuning to a LoRa
# channel used to leave the LoRa decoder gated off and strand VFO 0 outside the
# passband, and there was no way to obtain a second Listen control.
param([string]$BaseUrl = 'http://127.0.0.1:18081')

$ErrorActionPreference = 'Stop'
function Get-Api($path) { (Invoke-WebRequest -Uri "$BaseUrl$path" -UseBasicParsing).Content | ConvertFrom-Json }
function Post-Api($path, $body) {
  $json = if ($null -eq $body) { '{}' } else { $body | ConvertTo-Json -Compress }
  try {
    (Invoke-WebRequest -Uri "$BaseUrl$path" -Method Post -ContentType 'application/json' -Body $json -UseBasicParsing).Content | ConvertFrom-Json
  } catch {
    $reader = New-Object System.IO.StreamReader($_.Exception.Response.GetResponseStream())
    $reader.ReadToEnd() | ConvertFrom-Json
  }
}

$fails = 0
function Check($label, $condition, $detail) {
  if ($condition) { Write-Host "PASS  $label" } else { Write-Host "FAIL  $label -> $detail"; $script:fails++ }
}

Write-Host "--- baseline ---"
$before = Get-Api '/scan/status'
Write-Host "bank=$($before.range) running=$($before.running)"

Write-Host "--- tune to Meshtastic 906.875 MHz ---"
Post-Api '/device/frequency' @{ frequency_hz = 906875000 } | Out-Null
Start-Sleep -Seconds 3
$after = Get-Api '/scan/status'
$status = Get-Api '/device/status'
Write-Host "bank=$($after.range) center=$($status.center_freq_hz)"

$loraNeedles = @('ism 433', 'ism 915', '33cm', 'lora', '70cm')
$bankLower = "$($after.range)".ToLower()
$isLoraBank = $false
foreach ($needle in $loraNeedles) { if ($bankLower.Contains($needle)) { $isLoraBank = $true } }
Check 'bank follows the retune to a LoRa-capable bank' $isLoraBank $after.range
Check 'hardware actually retuned' ($status.center_freq_hz -eq 906875000) $status.center_freq_hz

Write-Host "--- VFO placement ---"
$vfos = Get-Api '/vfo/states'
$usableHalf = $status.sample_rate * 0.45
$stranded = @($vfos | Where-Object { [math]::Abs($_.frequency_hz - $status.center_freq_hz) -gt $usableHalf })
Check 'no VFO left outside the passband' ($stranded.Count -eq 0) "$($stranded.Count) stranded"

Write-Host "--- operator VFO slots ---"
$limit = (Get-Api '/scanner/max-vfos').max_vfos
Write-Host "max_vfos=$limit starting=$($vfos.Count)"
$added = 0
while ((Get-Api '/vfo/states').Count -lt $limit -and $added -lt 8) {
  Post-Api '/vfo/add' @{} | Out-Null
  Start-Sleep -Milliseconds 500
  $added++
}
$grown = Get-Api '/vfo/states'
Check 'multiple listening VFOs available' ($grown.Count -gt 1) "$($grown.Count) vfos"
Check 'reaches the configured limit' ($grown.Count -eq $limit) "$($grown.Count) of $limit"

$overflow = Post-Api '/vfo/add' @{}
Check 'add past the limit is rejected' ($overflow.ok -ne $true) ($overflow | ConvertTo-Json -Compress)

$victim = ($grown | Select-Object -Last 1).id
Post-Api "/vfo/$victim/remove" $null | Out-Null
Start-Sleep -Milliseconds 500
$shrunk = Get-Api '/vfo/states'
Check 'remove releases the slot' ($shrunk.Count -eq $grown.Count - 1) "$($shrunk.Count) vfos"

Write-Host "--- tune back to FM ---"
Post-Api '/device/frequency' @{ frequency_hz = 96100000 } | Out-Null
Start-Sleep -Seconds 3
$fm = Get-Api '/scan/status'
Check 'bank follows back to FM Broadcast' ($fm.range -eq 'FM Broadcast') $fm.range

Write-Host ""
if ($fails -eq 0) { Write-Host "ALL CHECKS PASSED" } else { Write-Host "$fails CHECK(S) FAILED" }
exit $fails
