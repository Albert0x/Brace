# Brace 的 PowerShell shell 集成。每次开标签由 Brace 写出并加载，手改会被覆盖。
#
# 每次显示提示符前上报两件事：
#   OSC 9;9;<cwd>                  供文件树跟随目录
#   OSC 6973;cmd;<nonce>;<退出码>;<命令>    刚执行完的命令，供命令历史
#
# 命令从 Get-History 取（只含真正执行过的），不靠记录按键。
# 加载方式是读文件再 Invoke-Expression，而不是 . brace.ps1——
# Windows PowerShell 默认执行策略会拦 .ps1 文件，很多机器上点源直接报错。

# nonce 由 Brace 经环境变量给出，屏幕上的程序拿不到，用来防止伪造历史。
# 读完立刻从环境里删掉，不让这个 shell 里启动的子进程继承
$global:__BraceNonce = $env:BRACE_HISTORY_NONCE
Remove-Item Env:BRACE_HISTORY_NONCE -ErrorAction SilentlyContinue

function global:prompt {
    # 必须是第一句：之后任何一条语句都会改写 $?
    $ok = $?
    $e = [char]27
    $p = (Get-Location).Path
    $out = ""

    $h = Get-History -Count 1
    $id = if ($h) { $h.Id } else { 0 }
    # 第一次只记下当前编号不上报：那一条是 Brace 注入的加载命令本身。
    # 编号没变 = 这次回车没执行新命令（空回车）
    if ($null -eq $global:__BraceLastHistoryId) {
        $global:__BraceLastHistoryId = $id
    } elseif ($id -ne $global:__BraceLastHistoryId -and $global:__BraceNonce) {
        $global:__BraceLastHistoryId = $id
        # 原生程序看 $LASTEXITCODE；cmdlet 失败时它可能是上一个原生程序留下的 0，记成 1
        $code = if ($ok) { 0 } elseif ($global:LASTEXITCODE) { $global:LASTEXITCODE } else { 1 }
        # OSC 里的控制字符会被终端丢掉，ESC / BEL 还会提前结束序列，所以要编码
        $c = $h.CommandLine -replace '%', '%25' -replace "`n", '%0A' -replace "`r", '%0D' `
            -replace "`t", '%09' -replace "$e", '%1B' -replace "`a", '%07'
        $out += "$e]6973;cmd;$global:__BraceNonce;$code;$c$e\"
    }
    # cwd 放在命令之后报：前端记历史时用的是「上一次报的目录」，
    # 这样 cd foo 记下的是它执行时所在的目录，而不是 foo
    $out += "$e]9;9;$p$e\"
    "${out}PS $p> "
}
