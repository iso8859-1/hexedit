; NSIS hooks for the Tauri installer (bundle.windows.nsis.installerHooks in tauri.conf.json).
; Adds "Open with HexEdit" to the Explorer context menu of every file. The installer is
; per-user, so the verb goes to HKCU and needs no administrator rights.
; On Windows 11 it is listed under "Show more options".

!define HEXEDIT_VERB_KEY "Software\Classes\*\shell\HexEdit"

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "${HEXEDIT_VERB_KEY}" "" "Open with HexEdit"
  WriteRegStr HKCU "${HEXEDIT_VERB_KEY}" "Icon" '"$INSTDIR\${MAINBINARYNAME}.exe",0'
  WriteRegStr HKCU "${HEXEDIT_VERB_KEY}\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "${HEXEDIT_VERB_KEY}"
!macroend
