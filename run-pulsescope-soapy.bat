@echo off
setlocal
set "POTHOSSDR=C:\Program Files\PothosSDR"
set "SDRPLAY_API=C:\Program Files\SDRplay\API\x64"
set "PULSESCOPE_HOME=%~dp0"

if not exist "%POTHOSSDR%\bin\SoapySDR.dll" (
  echo ERROR: PothosSDR / SoapySDR is not installed at "%POTHOSSDR%".
  exit /b 1
)
if not exist "%SDRPLAY_API%\sdrplay_api.dll" (
  echo ERROR: SDRplay API x64 is not installed at "%SDRPLAY_API%".
  exit /b 1
)
if not exist "%PULSESCOPE_HOME%src-tauri\target\release\pulsescope.exe" (
  echo ERROR: Soapy release binary not found. Build with:
  echo   cargo build --release --features soapysdr
  exit /b 1
)

set "PATH=%POTHOSSDR%\bin;%SDRPLAY_API%;%PATH%"
set "SOAPY_SDR_ROOT=%POTHOSSDR%"
set "PULSESCOPE_SOAPY_UTIL=%POTHOSSDR%\bin\SoapySDRUtil.exe"
cd /d "%PULSESCOPE_HOME%src-tauri\target\release"
start "PulseScope" "pulsescope.exe"
