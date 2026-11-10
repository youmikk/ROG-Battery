@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0mock-devices-launch.ps1"
if errorlevel 1 pause
