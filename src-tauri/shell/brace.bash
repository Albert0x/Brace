# Brace 的 bash shell 集成。每次开标签由 Brace 写出并 source，手改会被覆盖。
#
# 每次显示提示符前上报两件事：
#   OSC 9;9;<cwd>                  供文件树跟随目录
#   OSC 6973;cmd;<nonce>;<退出码>;<命令>    刚执行完的命令，供命令历史
#
# 命令从 bash 自己的历史里取，不靠记录按键：Tab 补全、方向键编辑、上翻历史
# 都会让按下的键和真正执行的命令对不上。代价是 bash 不记的它也不记——
# HISTCONTROL=ignorespace 时空格开头的命令不进历史，这正好是一条常用的隐私约定。

# nonce 由 Brace 经环境变量给出，屏幕上的程序拿不到，用来防止伪造历史。
# 读完立刻从环境里删掉，不让这个 shell 里启动的子进程继承
__brace_nonce=$BRACE_HISTORY_NONCE
unset BRACE_HISTORY_NONCE

# OSC 里的控制字符会被终端直接丢掉，ESC / BEL 还会提前结束序列，所以要编码。
# 纯参数展开，不开子进程——msys 下每 fork 一次都要几十毫秒
__brace_encode() {
  local s=${1//%/%25}
  s=${s//$'\n'/%0A}
  s=${s//$'\r'/%0D}
  s=${s//$'\t'/%09}
  s=${s//$'\e'/%1B}
  s=${s//$'\a'/%07}
  __brace_encoded=$s
}

__brace_prompt() {
  # 必须是第一句，否则 $? 就成了这个函数里某条命令的结果
  local ec=$?
  # 文件树要 Windows 路径。/d/foo 这种盘符挂载点直接换算成 D:/foo，不 fork；
  # 只有 /usr、/tmp 这类 msys 内部路径才去问 pwd -W。
  # 省下的这次 fork 正好抵掉下面取历史的那次——实测每次回车约 60ms
  local cwd
  if [[ $PWD =~ ^/([a-zA-Z])(/.*)?$ ]]; then
    cwd="${BASH_REMATCH[1]^^}:${BASH_REMATCH[2]:-/}"
  else
    cwd=$(pwd -W 2>/dev/null || pwd)
  fi

  local line num
  line=$(HISTTIMEFORMAT= builtin history 1)
  # 形如 "  123  git status"；被编辑过的条目编号后面带 *
  if [[ $line =~ ^[[:space:]]*([0-9]+)\*?[[:space:]]+(.*)$ ]]; then
    num=${BASH_REMATCH[1]}
    # 第一次只记下当前编号不上报：那一条是 Brace 注入的 source 本身。
    # 编号没变 = 这次回车没产生新的历史（空回车、被 ignoredups 忽略的重复命令）
    if [[ -z ${__brace_last_hist+x} ]]; then
      __brace_last_hist=$num
    elif [[ $num != "$__brace_last_hist" ]]; then
      __brace_last_hist=$num
      if [[ -n $__brace_nonce ]]; then
        __brace_encode "${BASH_REMATCH[2]}"
        printf '\e]6973;cmd;%s;%s;%s\e\\' "$__brace_nonce" "$ec" "$__brace_encoded"
      fi
    fi
  fi
  # cwd 放在命令之后报：前端记历史时用的是「上一次报的目录」，
  # 这样 cd foo 记下的是它执行时所在的目录，而不是 foo
  printf '\e]9;9;%s\e\\' "$cwd"
  # 把 $? 原样还回去，排在后面的 PROMPT_COMMAND 和用户的提示符逻辑看到的还是原值
  return $ec
}

# 接在用户已有的 PROMPT_COMMAND 前面，而不是覆盖它。重复 source 不重复挂
if [[ $PROMPT_COMMAND != *__brace_prompt* ]]; then
  PROMPT_COMMAND="__brace_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
