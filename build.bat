@echo off
title Building Internet Speed Meter (Production)
echo Setting up build environment...
set "ROOT=%~dp0"
set "PATH=%ROOT%w64devkit\w64devkit\bin;%USERPROFILE%\.cargo\bin;%PATH%"

echo Running Tauri build...
cd /d "%ROOT%"
npm run tauri build

if %ERRORLEVEL% EQU 0 (
    echo.
    echo ====================================================
    echo Build completed successfully!
    echo Output directory: src-tauri\target\release\bundle\
    echo ====================================================
) else (
    echo.
    echo Build failed with error code %ERRORLEVEL%.
)
pause
