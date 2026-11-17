@echo off
setlocal EnableExtensions

set "ROOT=%~dp0"
for %%I in ("%ROOT%") do set "ROOT=%%~fI"
pushd "%ROOT%" || exit /b 1

where.exe node >nul 2>&1 || goto :missing_node
where.exe npm >nul 2>&1 || goto :missing_npm
where.exe cargo >nul 2>&1 || goto :missing_cargo
where.exe rustup >nul 2>&1 || goto :missing_rustup
set "TARGET_FOUND="
for /f "delims=" %%T in ('rustup target list --installed') do if "%%T"=="x86_64-pc-windows-msvc" set "TARGET_FOUND=1"
if not defined TARGET_FOUND goto :missing_target

echo [1/4] Building the MCP HTTP sidecar...
cargo build --release -p agent-desktop --features mcp-streamable-http
if errorlevel 1 goto :failed

if not exist "apps\desktop\src-tauri\binaries" mkdir "apps\desktop\src-tauri\binaries"
copy /y "target\release\agent-desktop.exe" "apps\desktop\src-tauri\binaries\agent-desktop-x86_64-pc-windows-msvc.exe" >nul
if errorlevel 1 goto :failed

echo [2/4] Installing locked desktop dependencies...
pushd "apps\desktop" || goto :failed
call npm ci
if errorlevel 1 goto :failed_pushed

echo [3/4] Building the portable desktop app...
call npm run tauri -- build --no-bundle
if errorlevel 1 goto :failed_pushed
popd
if not exist "apps\desktop\src-tauri\target\release\agent-desktop-mcp-console.exe" goto :missing_portable_app

for /f "usebackq delims=" %%V in (`powershell -NoProfile -Command "(Get-Content 'apps\desktop\src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json).version"`) do set "APP_VERSION=%%V"
if not defined APP_VERSION goto :missing_version
if not exist "releases" mkdir "releases"
set "PORTABLE_DIR=releases\agent-desktop-mcp-portable_%APP_VERSION%_x64"
set "PORTABLE_ZIP=releases\agent-desktop-mcp-portable_%APP_VERSION%_x64.zip"
if not exist "%PORTABLE_DIR%" mkdir "%PORTABLE_DIR%"
copy /y "apps\desktop\src-tauri\target\release\agent-desktop-mcp-console.exe" "%PORTABLE_DIR%\agent-desktop-mcp-console.exe" >nul
if errorlevel 1 goto :failed
copy /y "apps\desktop\src-tauri\binaries\agent-desktop-x86_64-pc-windows-msvc.exe" "%PORTABLE_DIR%\agent-desktop.exe" >nul
if errorlevel 1 goto :failed
powershell -NoProfile -Command "Compress-Archive -Path '%PORTABLE_DIR%\agent-desktop-mcp-console.exe','%PORTABLE_DIR%\agent-desktop.exe' -DestinationPath '%PORTABLE_ZIP%' -Force"
if errorlevel 1 goto :failed

echo [4/4] Portable package ready:
echo %ROOT%\%PORTABLE_ZIP%
popd
exit /b 0

:missing_node
echo Missing Node.js. Install the Windows Node.js LTS release, then rerun this script.
goto :failed
:missing_npm
echo npm is missing. Repair the Node.js installation and make sure npm is on PATH.
goto :failed
:missing_cargo
echo Rust Cargo is missing. Install Rust with the MSVC toolchain, then rerun this script.
goto :failed
:missing_rustup
echo rustup is missing. Install Rust using rustup and rerun this script.
goto :failed
:missing_target
echo Rust target x86_64-pc-windows-msvc is missing. Run: rustup target add x86_64-pc-windows-msvc
goto :failed
:missing_version
echo Could not read the desktop app version from tauri.conf.json.
goto :failed
:missing_portable_app
echo Tauri did not produce the portable desktop executable.
goto :failed
:failed_pushed
popd
:failed
popd
exit /b 1
