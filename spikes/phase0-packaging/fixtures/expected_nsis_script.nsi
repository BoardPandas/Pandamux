; NSIS installer script generated for PandaMUX
; Mode: currentUser ($LOCALAPPDATA\Programs\PandaMUX)
; Formats: NSIS LZMA compressed installer

Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define PRODUCT_NAME "PandaMUX"
!define PRODUCT_VERSION "0.53.10"
!define PRODUCT_PUBLISHER "BoardPandas"
!define PRODUCT_WEB_SITE "https://pandamux.boardpandas.ai"
!define MAIN_BINARY_NAME "pandamux.exe"
!define SERVER_BINARY_NAME "pandamux-server.exe"
!define CLI_BINARY_NAME "pandamux-cli.exe"

InstallDir "$LOCALAPPDATA\Programs\PandaMUX"

Section "MainSection" SEC01
    SetOutPath "$INSTDIR"
    SetOverwrite on

    ; Multi-binary payload
    File "${STAGE_DIR}\pandamux.exe"
    File "${STAGE_DIR}\pandamux-server.exe"
    File "${STAGE_DIR}\pandamux-cli.exe"

    ; Bundled application resources
    SetOutPath "$INSTDIR\resources\themes"
    File /r "${STAGE_DIR}\resources\themes\*.*"
    SetOutPath "$INSTDIR\resources\sounds"
    File /r "${STAGE_DIR}\resources\sounds\*.*"
    SetOutPath "$INSTDIR\resources\icons"
    File /r "${STAGE_DIR}\resources\icons\*.*"

    ; Bundled Linux node binaries for remote bootstrap
    SetOutPath "$INSTDIR\resources\server\x86_64-unknown-linux-musl"
    File "${STAGE_DIR}\resources\server\x86_64-unknown-linux-musl\pandamux-server"
    File "${STAGE_DIR}\resources\server\x86_64-unknown-linux-musl\pandamux-cli"
    SetOutPath "$INSTDIR\resources\server\aarch64-unknown-linux-musl"
    File "${STAGE_DIR}\resources\server\aarch64-unknown-linux-musl\pandamux-server"
    File "${STAGE_DIR}\resources\server\aarch64-unknown-linux-musl\pandamux-cli"

    ; Shortcuts and Registry
    CreateDirectory "$SMPROGRAMS\PandaMUX"
    CreateShortcut "$SMPROGRAMS\PandaMUX\PandaMUX.lnk" "$INSTDIR\pandamux.exe"
    WriteUninstaller "$INSTDIR\uninstall.exe"

    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX" "DisplayName" "PandaMUX"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX" "DisplayIcon" "$INSTDIR\pandamux.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX" "DisplayVersion" "${PRODUCT_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX" "Publisher" "${PRODUCT_PUBLISHER}"
SectionEnd

Section "Uninstall"
    Delete "$SMPROGRAMS\PandaMUX\PandaMUX.lnk"
    RMDir "$SMPROGRAMS\PandaMUX"

    Delete "$INSTDIR\pandamux.exe"
    Delete "$INSTDIR\pandamux-server.exe"
    Delete "$INSTDIR\pandamux-cli.exe"
    Delete "$INSTDIR\uninstall.exe"

    RMDir /r "$INSTDIR\resources"
    RMDir "$INSTDIR"

    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX"
SectionEnd
