; Guitar Studio's Windows installer (Tauri's NSIS one) also installs the plugins where
; every DAW looks for them. They ship inside the app's folder (resources "plugins\",
; see tauri.windows.conf.json) and are copied out from there.

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$COMMONFILES64\VST3"
  CreateDirectory "$COMMONFILES64\CLAP"
  RMDir /r "$COMMONFILES64\VST3\Guitar Studio.vst3"
  RMDir /r "$COMMONFILES64\CLAP\Guitar Studio"
  CopyFiles /SILENT "$INSTDIR\plugins\Guitar Studio.vst3" "$COMMONFILES64\VST3"
  CopyFiles /SILENT "$INSTDIR\plugins\Guitar Studio" "$COMMONFILES64\CLAP"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  RMDir /r "$COMMONFILES64\VST3\Guitar Studio.vst3"
  RMDir /r "$COMMONFILES64\CLAP\Guitar Studio"
!macroend
