//! markdown_block：用 `<!-- dashi:begin/end id -->` 标记圈定的 Markdown 托管区块

pub(crate) fn block_begin(id: &str) -> String {
    format!("<!-- dashi:begin {} -->", id)
}

pub(crate) fn block_end(id: &str) -> String {
    format!("<!-- dashi:end {} -->", id)
}

pub(crate) fn extract_block<'a>(content: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let b = content.find(begin)?;
    let after = &content[b + begin.len()..];
    let e = after.find(end)?;
    Some(after[..e].trim())
}

pub(crate) fn upsert_block(content: &str, begin: &str, end: &str, block_content: &str) -> String {
    let block = format!("{}\n{}\n{}", begin, block_content.trim(), end);
    if let (Some(b), Some(e_start)) = (content.find(begin), content.find(end)) {
        if b <= e_start {
            let e = e_start + end.len();
            return format!("{}{}{}", &content[..b], block, &content[e..]);
        }
        // 结构校验在 engine 中会先拒绝逆序标记；这里也不再把坏文档当空文档追加，
        // 避免该 helper 被未来写入口单独调用时静默扩大损坏范围。
        return content.to_string();
    }
    if content.contains(begin) || content.contains(end) {
        return content.to_string();
    }
    if content.trim().is_empty() {
        format!("{}\n", block)
    } else {
        format!("{}\n\n{}\n", content.trim_end(), block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_block_append_and_replace() {
        let s = upsert_block(
            "# 我的笔记\n\n已有内容。\n",
            "<!-- b -->",
            "<!-- e -->",
            "你好",
        );
        assert!(s.contains("已有内容。"));
        assert!(s.contains("<!-- b -->\n你好\n<!-- e -->"));
        let s2 = upsert_block(&s, "<!-- b -->", "<!-- e -->", "世界");
        assert!(s2.contains("世界"));
        assert!(!s2.contains("你好"));
        assert_eq!(extract_block(&s2, "<!-- b -->", "<!-- e -->"), Some("世界"));
    }

    #[test]
    fn upsert_block_empty_content_produces_block_only() {
        let s = upsert_block("", "<!-- b -->", "<!-- e -->", "内容");
        assert_eq!(s, "<!-- b -->\n内容\n<!-- e -->\n");
    }

    #[test]
    fn upsert_block_reversed_or_partial_markers_leave_content_unchanged() {
        // 标记顺序颠倒或残缺（文件被手工改坏）：不得替换，保留原文
        let broken = "<!-- e -->\nxxx\n<!-- b -->\n";
        let s = upsert_block(broken, "<!-- b -->", "<!-- e -->", "新");
        assert_eq!(s, broken);
        let partial = "<!-- b -->\nxxx\n";
        assert_eq!(
            upsert_block(partial, "<!-- b -->", "<!-- e -->", "新"),
            partial
        );
    }
}
