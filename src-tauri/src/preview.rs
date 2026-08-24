use serde::Serialize;

// ---------- 文件预览 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilePreview {
    kind: String,    // text | image | binary | toolarge
    content: String, // text: 文本内容；image: data URI；其他: 空
    size: u64,
    encoding: String, // text: 原始编码，保存时按它写回；其他: 空
}

// 编码标签。UTF-16 由我们自己处理（encoding_rs 不支持 encode 到 UTF-16），
// 其余走 encoding_rs 的规范名（"UTF-8" / "GBK" / "Shift_JIS" / "windows-1252"…）
const ENC_UTF8: &str = "UTF-8";
const ENC_UTF8_BOM: &str = "UTF-8-BOM";
const ENC_UTF16LE: &str = "UTF-16LE";
const ENC_UTF16BE: &str = "UTF-16BE";

// 按 BOM 判编码；返回 (编码标签, BOM 长度)
fn sniff_bom(b: &[u8]) -> Option<(&'static str, usize)> {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((ENC_UTF8_BOM, 3))
    } else if b.starts_with(&[0xFF, 0xFE]) {
        Some((ENC_UTF16LE, 2))
    } else if b.starts_with(&[0xFE, 0xFF]) {
        Some((ENC_UTF16BE, 2))
    } else {
        None
    }
}

// UTF-16 解码（BOM 已剥离）。奇数个字节说明文件截断，末尾半个码元直接丢掉
fn decode_utf16(body: &[u8], little: bool) -> String {
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| {
            if little {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

// 字节 → (文本, 编码标签)。判不出文本则返回 None（交给调用方当二进制处理）。
// 顺序：BOM → NUL 探测 → 严格 UTF-8 → chardetng 嗅探
pub(crate) fn decode_text(bytes: &[u8]) -> Option<(String, String)> {
    if let Some((enc, skip)) = sniff_bom(bytes) {
        let body = &bytes[skip..];
        return Some(match enc {
            ENC_UTF16LE => (decode_utf16(body, true), enc.into()),
            ENC_UTF16BE => (decode_utf16(body, false), enc.into()),
            _ => (String::from_utf8_lossy(body).into_owned(), enc.into()),
        });
    }
    // NUL 字节基本可以断定是二进制。必须放在 BOM 判断之后——UTF-16 里的 ASCII
    // 字符高位字节全是 0x00，先查 NUL 会把 UTF-16 文本全部误杀
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return None;
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some((s.to_string(), ENC_UTF8.into()));
    }
    // 不是合法 UTF-8：嗅探（对 GBK/Big5/Shift_JIS 这些中日韩编码识别率还行）。
    // ISO-2022-JP 用转义序列表示，字节全在 ASCII 范围内，合法 UTF-8 那步就已经拦下了，
    // 走到这儿再允许它只会增加误判，故 Deny
    use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
    let mut det = EncodingDetector::new(Iso2022JpDetection::Deny);
    det.feed(bytes, true);
    let enc = det.guess(None, Utf8Detection::Allow);
    let (text, _, had_errors) = enc.decode(bytes);
    // 猜的编码解出来还是一堆替换字符，说明根本不是文本，别硬凑
    if had_errors && text.matches('\u{FFFD}').count() * 20 > text.chars().count() {
        return None;
    }
    Some((text.into_owned(), enc.name().to_string()))
}

#[tauri::command]
pub(crate) fn read_file(path: String) -> FilePreview {
    let empty = |kind: &str, size: u64| FilePreview {
        kind: kind.into(),
        content: String::new(),
        size,
        encoding: String::new(),
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let is_img = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg"
    );
    if is_img {
        if size > 5_000_000 {
            return empty("toolarge", size);
        }
        if let Ok(bytes) = std::fs::read(&path) {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let mime: String = match ext.as_str() {
                "svg" => "image/svg+xml".into(),
                "jpg" | "jpeg" => "image/jpeg".into(),
                "ico" => "image/x-icon".into(),
                e => format!("image/{}", e),
            };
            return FilePreview {
                kind: "image".into(),
                content: format!("data:{};base64,{}", mime, b64),
                size,
                encoding: String::new(),
            };
        }
        return empty("binary", size);
    }

    // 文本：超过 2MB 不预览。编码不限 UTF-8——GBK / UTF-16 也照样认，
    // 并把识别出的编码带回前端，保存时原样写回，不静默转码
    if size > 2_000_000 {
        return empty("toolarge", size);
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return empty("binary", size),
    };
    match decode_text(&bytes) {
        Some((text, encoding)) => FilePreview {
            kind: "text".into(),
            content: text,
            size,
            encoding,
        },
        None => empty("binary", size),
    }
}

// 按指定编码把文本编码成字节。encoding 为空或未知时退回 UTF-8。
// 返回 None 表示该编码无法表达这段文本（例如往 GBK 里塞 emoji），由调用方报错，
// 绝不静默用 '?' 替换掉用户的字符
fn encode_text(content: &str, encoding: &str) -> Option<Vec<u8>> {
    match encoding {
        ENC_UTF8_BOM => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(content.as_bytes());
            Some(v)
        }
        ENC_UTF16LE | ENC_UTF16BE => {
            let little = encoding == ENC_UTF16LE;
            let mut v = if little {
                vec![0xFF, 0xFE]
            } else {
                vec![0xFE, 0xFF]
            };
            for u in content.encode_utf16() {
                v.extend_from_slice(&if little {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                });
            }
            Some(v)
        }
        "" | ENC_UTF8 => Some(content.as_bytes().to_vec()),
        name => match encoding_rs::Encoding::for_label(name.as_bytes()) {
            Some(enc) => {
                let (bytes, _, had_errors) = enc.encode(content);
                if had_errors {
                    None
                } else {
                    Some(bytes.into_owned())
                }
            }
            None => Some(content.as_bytes().to_vec()),
        },
    }
}

#[tauri::command]
pub(crate) fn write_file(
    path: String,
    content: String,
    encoding: Option<String>,
) -> Result<(), String> {
    let enc = encoding.unwrap_or_default();
    let bytes = encode_text(&content, &enc)
        .ok_or_else(|| format!("有字符无法用原编码 {} 保存，请先把文件转成 UTF-8", enc))?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    // ----- 编码识别与往返 -----

    const CN: &str = "老王在终端里敲下了一行命令，然后盯着输出发呆了整整三分钟。\
                      这段文本要足够长，编码嗅探才有足够的统计样本可用。";

    #[test]
    fn detects_plain_utf8() {
        let (text, enc) = decode_text(CN.as_bytes()).unwrap();
        assert_eq!(text, CN);
        assert_eq!(enc, "UTF-8");
    }

    #[test]
    fn detects_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(CN.as_bytes());
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, CN, "BOM 不能混进正文");
        assert_eq!(enc, "UTF-8-BOM");
    }

    #[test]
    fn detects_utf16le_despite_embedded_nul_bytes() {
        // PowerShell 5.1 的 Out-File 默认就是这个格式；ASCII 字符高位全是 0x00，
        // 二进制探测必须让位于 BOM 判断
        let mut bytes = vec![0xFF, 0xFE];
        for u in "hello 世界".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, "hello 世界");
        assert_eq!(enc, "UTF-16LE");
    }

    #[test]
    fn detects_utf16be() {
        let mut bytes = vec![0xFE, 0xFF];
        for u in "hello 世界".encode_utf16() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, "hello 世界");
        assert_eq!(enc, "UTF-16BE");
    }

    #[test]
    fn detects_gbk_chinese_text() {
        let (bytes, _, err) = encoding_rs::GBK.encode(CN);
        assert!(!err, "测试数据本身应能用 GBK 表示");
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, CN, "中文 GBK 文件不该被当成二进制或乱码");
        assert_ne!(enc, "UTF-8");
    }

    #[test]
    fn treats_nul_containing_data_as_binary() {
        assert!(decode_text(&[0x00, 0x01, 0x02, b'a']).is_none());
    }

    #[test]
    fn roundtrips_every_detected_encoding() {
        // 识别出来的编码必须能原样写回去，否则保存会静默转码
        for original in [
            {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice(CN.as_bytes());
                v
            },
            {
                let mut v = vec![0xFF, 0xFE];
                for u in CN.encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            },
            {
                let mut v = vec![0xFE, 0xFF];
                for u in CN.encode_utf16() {
                    v.extend_from_slice(&u.to_be_bytes());
                }
                v
            },
            encoding_rs::GBK.encode(CN).0.into_owned(),
            CN.as_bytes().to_vec(),
        ] {
            let (text, enc) = decode_text(&original).unwrap();
            let written =
                encode_text(&text, &enc).unwrap_or_else(|| panic!("编码 {} 无法写回", enc));
            let (again, enc2) = decode_text(&written).unwrap();
            assert_eq!(again, text, "编码 {} 往返后内容变了", enc);
            assert_eq!(enc2, enc, "编码 {} 往返后编码变了", enc);
        }
    }

    #[test]
    fn refuses_to_save_characters_the_original_encoding_cannot_hold() {
        // GBK 装不下 emoji：宁可报错，也不能用 '?' 悄悄替换掉用户的字符
        assert!(encode_text("🦀", "GBK").is_none());
        assert!(encode_text("🦀", "UTF-8").is_some());
        assert!(encode_text("🦀", ENC_UTF16LE).is_some());
    }

    #[test]
    fn unknown_encoding_label_falls_back_to_utf8() {
        assert_eq!(encode_text("abc", "no-such-encoding").unwrap(), b"abc");
        assert_eq!(encode_text("abc", "").unwrap(), b"abc");
    }
}
