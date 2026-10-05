@echo off
setlocal enabledelayedexpansion

title OpenHeart - Precision Program Graph Studio

set PORT=8080
if not "%~1"=="" set PORT=%~1

echo ===============================================================================
echo   OpenHeart - Precision Program Graph Studio
echo ===============================================================================
echo.

:: Change directory to repo root where this script resides
cd /d "%~dp0"

:: 1. Check if Cargo/Rust is installed
where cargo >nul 2>&1 && (
    echo [OpenHeart] Rust/Cargo detected. Launching native OpenHeart server on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start http://localhost:%PORT%
    cargo run -- server %PORT%
    goto :eof
)

:: 2. Fall back to Python if Cargo is not installed
where python >nul 2>&1 && (
    echo [OpenHeart] Python detected. Launching OpenHeart Web Studio on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start http://localhost:%PORT%
    python -m http.server %PORT% --directory web
    goto :eof
)

where py >nul 2>&1 && (
    echo [OpenHeart] Python launcher (py) detected. Launching OpenHeart Web Studio on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start http://localhost:%PORT%
    py -m http.server %PORT% --directory web
    goto :eof
)

where python3 >nul 2>&1 && (
    echo [OpenHeart] Python3 detected. Launching OpenHeart Web Studio on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start http://localhost:%PORT%
    python3 -m http.server %PORT% --directory web
    goto :eof
)

:: 3. If neither Cargo nor Python is found, open index.html directly
echo [OpenHeart] Neither Cargo nor Python found. Opening OpenHeart Web Studio directly...
start "" "%~dp0web\index.html"
echo [OpenHeart] Opened web\index.html in default browser.
pause
