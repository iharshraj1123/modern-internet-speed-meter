@echo off
title Patch MSIX Manifest (Auto-Startup Task)
set "ROOT=%~dp0"
cd /d "%ROOT%"
echo Running MSIX Auto-Patcher...
node scripts/patch-msix.cjs
echo.
pause
