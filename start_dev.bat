@echo off
title Starting Internet Speed Meter (Development Mode)
echo Setting up environment...
set "ROOT=%~dp0"
set "PATH=%ROOT%w64devkit\w64devkit\bin;%USERPROFILE%\.cargo\bin;%PATH%"

echo Starting Tauri dev server...
cd /d "%ROOT%"
npm run tauri dev
pause
