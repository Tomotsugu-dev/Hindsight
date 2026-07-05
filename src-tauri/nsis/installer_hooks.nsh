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

    ; ── 迁移清理：历史版本 installMode 是 "both"，可能存在“所有用户”(per-machine,
    ; Program Files) 安装与本 per-user 安装并存。两份并存时更新器只更新其一、
    ; 快捷方式却可能指向另一份——用户“更新完重开还是旧版本”。这里发现 HKLM
    ; 残留就静默运行它的卸载器（需要一次 UAC 确认；拒绝则跳过并提示手动卸载）。
    ; 卸载器的“是否删除用户数据”弹窗带 /SD IDNO，静默模式自动选“否”；
    ; 数据库/截图都在 %APPDATA%\hindsight，不在安装目录，绝不受影响。
    ClearErrors
    ReadRegStr $0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\hindsight" "UninstallString"
    IfErrors machine_copy_done
    StrCmp $0 "" machine_copy_done
    DetailPrint "Removing legacy all-users installation..."
    ; UninstallString 形如 "C:\Program Files\hindsight\uninstall.exe"（带引号），
    ; ShellExecute 的 file 参数不吃引号，剥掉首尾各一个字符
    StrCpy $1 $0 "" 1
    StrCpy $1 $1 -1
    ClearErrors
    ExecShellWait "runas" "$1" "/S"
    IfErrors 0 +2
    DetailPrint "Legacy copy NOT removed (elevation declined). Please uninstall the old 'hindsight' under Program Files manually."
    ; 卸载器会自我复制到临时目录后立即返回，留 2s 让它删完全机快捷方式，
    ; 再让本安装继续写当前用户快捷方式，避免先写后删的交错
    Sleep 2000
    machine_copy_done:
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
