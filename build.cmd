@echo off
rem build.cmd -- build rog-battery.exe (tray app, no console window on double-click)
rem Uses the C# compiler that ships with Windows (.NET Framework 4.x).
rem No .NET SDK, no Visual Studio, no Rust, no Python needed.

setlocal
set CSC=%WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe
if not exist "%CSC%" set CSC=%WINDIR%\Microsoft.NET\Framework\v4.0.30319\csc.exe
if not exist "%CSC%" (
  echo ERROR: csc.exe not found. .NET Framework 4.x is required.
  exit /b 1
)

cd /d "%~dp0"
echo Building with: %CSC%
rem /target:winexe  -> GUI subsystem, so double-click shows NO console window.
rem                     When launched from cmd/PowerShell we AttachConsole() to the
rem                     parent so --once/--watch/--help still print.
"%CSC%" /nologo /target:winexe /platform:x64 /optimize+ ^
  /r:System.dll /r:System.Drawing.dll ^
  /out:rog-battery.exe rog-battery.cs tray.cs settings.cs devices.cs
if errorlevel 1 (
  echo BUILD FAILED
  exit /b 1
)
echo.
echo Build OK: rog-battery.exe
for %%F in (rog-battery.exe) do echo   size: %%~zF bytes
endlocal
