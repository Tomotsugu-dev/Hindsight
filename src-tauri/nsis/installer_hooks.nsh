; Hindsight NSIS installer hooks.
; Tauri inserts these macros inside the auto-generated Section "Uninstall"
; (run via !ifmacrodef so missing hooks are silently skipped).

; 安装写文件之前：把在跑的 hindsight.exe 杀掉。Hindsight 是托盘常驻应用，
; 覆盖安装（升级）时旧进程握着 hindsight.exe 的文件锁，NSIS 写入会报
; "Error opening file for writing"。Tauri 模板自带的"关闭运行中应用"检测
; 对托盘应用不可靠，这里强杀兜底。
!macro NSIS_HOOK_PREINSTALL
    DetailPrint "Stopping hindsight.exe before install..."
    nsExec::Exec '"taskkill" /F /IM hindsight.exe /T'
    Sleep 1000
!macroend

; 卸载主流程开始前：把 hindsight.exe 杀掉，避免它握着安装目录里的可执行文件
; 或 %APPDATA% 里的 SQLite/截图，导致后续 RMDir 删不干净。
; nsExec::Exec 是静默执行（无黑窗一闪），taskkill /F /T 强杀含子进程。
!macro NSIS_HOOK_PREUNINSTALL
    DetailPrint "Stopping hindsight.exe before uninstall..."
    nsExec::Exec '"taskkill" /F /IM hindsight.exe /T'
    Sleep 1000
!macroend

; 主流程把程序文件 / 注册表 / 快捷方式都删完后：问用户是否同时清掉用户数据。
; - SetShellVarContext current 让 $APPDATA 解析为当前用户的 AppData\Roaming
;   (安装是 perMachine 时 SHCTX 默认 all_users，$APPDATA 会变成 ProgramData)
; - /SD IDNO 让 silent uninstall 默认选 No，避免 msiexec /qn 等场景自动删数据
; - 选 Yes 走 RMDir /r 递归删 %APPDATA%\hindsight
!macro NSIS_HOOK_POSTUNINSTALL
    SetShellVarContext current
    MessageBox MB_YESNO|MB_ICONQUESTION \
        "Also delete all user data (database, screenshots, login info)?$\n$\nThis cannot be undone." \
        /SD IDNO IDYES delete_user_data
    Goto skip_user_data
    delete_user_data:
        RMDir /r "$APPDATA\hindsight"
    skip_user_data:
!macroend
