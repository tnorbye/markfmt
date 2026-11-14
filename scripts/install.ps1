# Copyright 2026 Tor Norbye
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Installer for markfmt Windows release archives. Fetches the x86_64-pc-windows-msvc archive and
# SHA256SUMS.txt, verifies the checksum, and installs markfmt.exe into $MARKFMT_INSTALL_DIR.

& {
    $ErrorActionPreference = "Stop"

    $Repo = if ($env:MARKFMT_REPO) { $env:MARKFMT_REPO } else { "tnorbye/markfmt" }
    $ReleaseTag = "__MARKFMT_RELEASE_TAG__"
    $InstallDir = if ($env:MARKFMT_INSTALL_DIR) { $env:MARKFMT_INSTALL_DIR } else { Join-Path $HOME ".local\bin" }
    $BaseUrl = if ($env:MARKFMT_BASE_URL) { $env:MARKFMT_BASE_URL.TrimEnd("/") } else { "https://github.com/$Repo/releases/download" }

    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch {
    }

    if ($env:MARKFMT_VERSION) {
        $Tag = if ($env:MARKFMT_VERSION.StartsWith("v")) { $env:MARKFMT_VERSION } else { "v$($env:MARKFMT_VERSION)" }
    } elseif ($ReleaseTag -and ($ReleaseTag -notlike "__MARKFMT_*")) {
        $Tag = $ReleaseTag
    } else {
        $LatestUrl = "https://github.com/$Repo/releases/latest"
        $Response = Invoke-WebRequest -Uri $LatestUrl -UseBasicParsing
        $FinalUrl = $null
        if ($Response.BaseResponse -and $Response.BaseResponse.ResponseUri) {
            $FinalUrl = $Response.BaseResponse.ResponseUri.AbsoluteUri
        } elseif ($Response.BaseResponse -and $Response.BaseResponse.RequestMessage -and $Response.BaseResponse.RequestMessage.RequestUri) {
            $FinalUrl = $Response.BaseResponse.RequestMessage.RequestUri.AbsoluteUri
        }
        if (-not $FinalUrl -or $FinalUrl -notmatch "/releases/tag/(v[^/]+)$") {
            throw "Could not resolve latest release tag from $LatestUrl"
        }
        $Tag = $Matches[1]
    }

    $Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    if ($Arch -ne [System.Runtime.InteropServices.Architecture]::X64 -and $env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
        throw "Unsupported Windows architecture: $Arch (only x86_64-pc-windows-msvc binaries are published)"
    }

    $Version = $Tag.TrimStart("v")
    $Target = "x86_64-pc-windows-msvc"
    $Stage = "markfmt-v$Version-$Target"
    $Archive = "$Stage.zip"

    $TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("markfmt-install-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null

    try {
        $ZipPath = Join-Path $TmpDir $Archive
        $SumsPath = Join-Path $TmpDir "SHA256SUMS.txt"

        Write-Host "Downloading $Archive..."
        Invoke-WebRequest -Uri "$BaseUrl/$Tag/$Archive" -OutFile $ZipPath -UseBasicParsing
        Invoke-WebRequest -Uri "$BaseUrl/$Tag/SHA256SUMS.txt" -OutFile $SumsPath -UseBasicParsing

        $Expected = $null
        foreach ($Line in (Get-Content -Path $SumsPath)) {
            if ($Line -match "^([0-9a-fA-F]{64})\s+\*?(.*)$" -and $Matches[2].Trim() -eq $Archive) {
                $Expected = $Matches[1].ToLowerInvariant()
                break
            }
        }
        if (-not $Expected) {
            throw "Missing checksum for $Archive in SHA256SUMS.txt"
        }

        $Actual = (Get-FileHash -Algorithm SHA256 -Path $ZipPath).Hash.ToLowerInvariant()
        if ($Actual -ne $Expected) {
            throw "Checksum verification failed for ${Archive}: expected $Expected, got $Actual"
        }

        Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force
        $BinSource = Join-Path (Join-Path $TmpDir $Stage) "markfmt.exe"
        if (-not (Test-Path -Path $BinSource -PathType Leaf)) {
            throw "Archive $Archive did not contain $Stage\markfmt.exe"
        }

        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
        $BinDest = Join-Path $InstallDir "markfmt.exe"
        Copy-Item -Path $BinSource -Destination $BinDest -Force

        Write-Host "Installed markfmt $Version to $BinDest"
        $PathEntries = ($env:Path -split ";") | ForEach-Object { $_.Trim().TrimEnd("\") }
        $NormalizedInstallDir = $InstallDir.Trim().TrimEnd("\")
        if ($PathEntries -notcontains $NormalizedInstallDir) {
            Write-Host "Add it to your PATH with:`n  `$env:Path = `"$InstallDir;`$env:Path`""
        }
    } finally {
        if (Test-Path -Path $TmpDir) {
            Remove-Item -Recurse -Force -Path $TmpDir
        }
    }
}
