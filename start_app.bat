@echo off
title Launching Internet Speed Meter
set "ROOT=%~dp0"
set "EXE=%ROOT%src-tauri\target\release\internet-speed-meter.exe"

if exist "%EXE%" (
    echo Starting Internet Speed Meter...
    start "" "%EXE%"
) else (
    echo [ERROR] Executable not found at:
    echo %EXE%
    echo Please run build.bat first to build the application.
    pause
)
