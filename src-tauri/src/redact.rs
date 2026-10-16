// 识别「看起来带着密钥」的文本。
//
// 命令历史用它决定一条命令存不存（0.3.0），AI 命令建议发送终端输出前用它脱敏（0.4.0）。
// 这是一条安全承诺，所以规则集中在这一个地方，并且每条都有测试——
// 靠肉眼看着「应该能拦住」不算数。
//
// 宁可多拦：一条正常命令没进历史，用户顶多重敲一遍；
// 一个 token 落进历史文件，就是明文躺在磁盘上。

use regex::Regex;
use std::sync::OnceLock;

fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            // OpenAI / Anthropic / DeepSeek 等的 key：sk-、sk-ant-、sk-proj-……
            r"\bsk-[A-Za-z0-9_\-]{16,}",
            // GitHub token
            r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}",
            // AWS access key id
            r"\bAKIA[0-9A-Z]{16}\b",
            // HTTP 头里的凭据
            r"(?i)\bbearer\s+[A-Za-z0-9._\-~+/=]{8,}",
            r"(?i)\bauthorization\s*:",
            // 往 *_KEY / *_TOKEN / *_SECRET / *_PASSWORD 这类变量里赋值：
            // export / set / $env: 都覆盖到，只看「名字 = 非空值」
            r"(?i)\b[A-Z0-9_]*(KEY|TOKEN|SECRET|PASSWORD|PASSWD|PWD)\s*=\s*\S",
            // 命令行参数形式：--password xxx、--token=xxx、-p'xxx' 这种太泛，不管
            r"(?i)--(password|passwd|token|secret|api-?key)(\s+|=)\S",
            // URL 里内嵌的账号密码：https://user:pass@host
            r"[A-Za-z][A-Za-z0-9+.\-]*://[^\s/:@]+:[^\s/@]+@",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("内置正则写错了"))
        .collect()
    })
}

pub(crate) fn looks_secret(text: &str) -> bool {
    patterns().iter().any(|re| re.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_secrets() {
        for s in [
            "export ANTHROPIC_AUTH_TOKEN=sk-ant-api03-abcdefghijklmnop",
            "curl -H 'Authorization: Bearer abcdefgh12345678' https://x",
            "curl -H \"authorization: token\" https://x",
            "$env:OPENAI_API_KEY = 'whatever'",
            "set DEEPSEEK_API_KEY=abc",
            "git clone https://ghp_abcdefghijklmnopqrstuvwx@github.com/a/b",
            "git remote add o https://alice:hunter2@example.com/r.git",
            "aws configure set aws_access_key_id AKIAABCDEFGHIJKLMNOP",
            "mysql --password=hunter2 -u root",
            "gh auth login --with-token --token abc",
            "echo sk-proj-1234567890abcdefXYZ",
            "export DB_PASSWORD=x",
        ] {
            assert!(looks_secret(s), "没拦住：{s}");
        }
    }

    #[test]
    fn lets_ordinary_commands_through() {
        for s in [
            "git status",
            "git commit -m \"fix: token refresh\"",
            "cargo test --manifest-path src-tauri/Cargo.toml",
            "echo $PATH",
            "ls -la ~/.ssh",
            "cd D:/Desktop/HyperTerminal",
            "npm run task-key",
            "grep -rn password src/",
            "ssh user@example.com",
            "Get-ChildItem Env:",
            "skip-this --flag",
        ] {
            assert!(!looks_secret(s), "误拦：{s}");
        }
    }
}
