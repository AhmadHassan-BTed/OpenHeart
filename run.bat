@echo off
setlocal EnableExtensions DisableDelayedExpansion

title OpenHeart - Precision Program Graph Studio

:: Change directory to repo root where this script resides
cd /d "%~dp0"

:: Optional local overrides (see .env.example). Variables already set in the calling
:: environment win over the file.
if exist ".env" (
    for /f "usebackq eol=# tokens=1* delims==" %%A in (".env") do (
        if not defined %%A set "%%A=%%~B"
    )
)

set "PORT=8080"
if defined OPENHEART_PORT set "PORT=%OPENHEART_PORT%"
if not "%~1"=="" set "PORT=%~1"

set "URL=http://localhost:%PORT%/"
if defined OPENHEART_API_TOKEN set "URL=http://localhost:%PORT%/?token=%OPENHEART_API_TOKEN%"

echo ===============================================================================
echo   OpenHeart - Precision Program Graph Studio
echo ===============================================================================
echo.

:: 1. Check if Cargo/Rust is installed. The server binds 127.0.0.1 unless
::    OPENHEART_HOST says otherwise.
where cargo >nul 2>&1 && (
    echo [OpenHeart] Rust/Cargo detected. Launching native OpenHeart server on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start "" "%URL%"
    cargo run -- server --port %PORT%
    goto :eof
)

:: 2. Fall back to Python if Cargo is not installed (static studio only, loopback only)
where python >nul 2>&1 && (
    echo [OpenHeart] Python detected. Launching OpenHeart Web Studio on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start "" "http://localhost:%PORT%/"
    python -m http.server %PORT% --bind 127.0.0.1 --directory web
    goto :eof
)

where py >nul 2>&1 && (
    echo [OpenHeart] Python launcher ^(py^) detected. Launching OpenHeart Web Studio on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start "" "http://localhost:%PORT%/"
    py -m http.server %PORT% --bind 127.0.0.1 --directory web
    goto :eof
)

where python3 >nul 2>&1 && (
    echo [OpenHeart] Python3 detected. Launching OpenHeart Web Studio on port %PORT%...
    echo [OpenHeart] Opening http://localhost:%PORT% in your default browser...
    start "" "http://localhost:%PORT%/"
    python3 -m http.server %PORT% --bind 127.0.0.1 --directory web
    goto :eof
)

:: 3. If neither Cargo nor Python is found, open index.html directly
echo [OpenHeart] Neither Cargo nor Python found. Opening OpenHeart Web Studio directly...
start "" "%~dp0web\index.html"
echo [OpenHeart] Opened web\index.html in default browser.
pause
