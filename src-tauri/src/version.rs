//! 极简语义版本比较（semver 子集）：仅用于「是否有更新」判断。
//! 支持可选的 v 前缀、`-prerelease` 后缀，忽略 `+build` 元数据。
//! 由 fastctx 与 dsh 两个 npm 包版本检测共用（唯一实现，避免逻辑分叉）。

/// prerelease 标识符：数字或字母数字（semver 优先级：数字 < 字母数字）
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Alpha(String),
}

/// 解析后的版本：核心三段 + 可选 prerelease 标识符列表
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Vec<PreId>,
}

/// 解析 "1.2.3" / "v1.2.3" / "0.1.0-rc.6" / "1.2.3+build.5" → Version。
/// 语义版本规范要求核心段恰好 3 段数字；解析失败返回 None
pub fn parse_version(v: &str) -> Option<Version> {
    let s = v.trim().trim_start_matches(['v', 'V']);
    // 忽略 +build 元数据（不影响优先级比较）
    let s = s.split('+').next().unwrap_or(s);
    // 核心段与 prerelease 以第一个 '-' 分隔
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (s, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // 核心段多于 3 段
    }
    let pre = match pre {
        None => Vec::new(),
        Some(p) => {
            let mut ids = Vec::new();
            for id in p.split('.') {
                if id.is_empty() {
                    return None;
                }
                ids.push(if id.chars().all(|c| c.is_ascii_digit()) {
                    PreId::Num(id.parse().ok()?)
                } else {
                    PreId::Alpha(id.to_string())
                });
            }
            ids
        }
    };
    Some(Version { major, minor, patch, pre })
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        for (a, b) in [(self.major, other.major), (self.minor, other.minor), (self.patch, other.patch)] {
            match a.cmp(&b) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        // 核心相同：无 prerelease 的发布版 > 有 prerelease 的候选版
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            (false, false) => {}
        }
        // 逐标识符比较：数字 < 字母数字；数字按数值、字母数字按 ASCII 字典序
        for (a, b) in self.pre.iter().zip(&other.pre) {
            let ord = match (a, b) {
                (PreId::Num(x), PreId::Num(y)) => x.cmp(y),
                (PreId::Num(_), PreId::Alpha(_)) => Ordering::Less,
                (PreId::Alpha(_), PreId::Num(_)) => Ordering::Greater,
                (PreId::Alpha(x), PreId::Alpha(y)) => x.cmp(y),
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        // 前缀相同：标识符少的优先级低（rc.6 < rc.6.1）
        self.pre.len().cmp(&other.pre.len())
    }
}

/// cur < latest 才算有更新；任一侧解析失败按无更新处理
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        assert_eq!(
            parse_version("1.2.3"),
            Some(Version { major: 1, minor: 2, patch: 3, pre: vec![] })
        );
        assert_eq!(
            parse_version("v1.2.3"),
            Some(Version { major: 1, minor: 2, patch: 3, pre: vec![] })
        );
        assert_eq!(
            parse_version(" 1.2.3 "),
            Some(Version { major: 1, minor: 2, patch: 3, pre: vec![] })
        );
    }

    #[test]
    fn parse_prerelease_and_build() {
        // dsh 实际版本形如 0.1.0-rc.6
        assert_eq!(
            parse_version("0.1.0-rc.6"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: vec![PreId::Alpha("rc".into()), PreId::Num(6)],
            })
        );
        // +build 被忽略，不影响解析结果
        assert_eq!(parse_version("1.2.3+build.5"), parse_version("1.2.3"));
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("a.b.c"), None);
        assert_eq!(parse_version("1.2.x"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1.2.3-"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn is_newer_core() {
        assert!(is_newer("1.2.3", "1.2.2"));
        assert!(is_newer("1.3.0", "1.2.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        // 相同 / 已装更新 → 无更新
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2.2", "1.2.3"));
        // 任一侧解析失败 → 无更新
        assert!(!is_newer("v1.2.3", "abc"));
        assert!(!is_newer("", "1.2.3"));
    }

    #[test]
    fn is_newer_prerelease() {
        // 发布版 > 同版本 prerelease（rc.6 有新 0.1.0 稳定版即视为可更新）
        assert!(is_newer("0.1.0", "0.1.0-rc.6"));
        assert!(!is_newer("0.1.0-rc.6", "0.1.0"));
        // prerelease 之间比较
        assert!(is_newer("0.1.0-rc.7", "0.1.0-rc.6"));
        assert!(is_newer("0.1.0-rc.10", "0.1.0-rc.9"));
        assert!(is_newer("0.1.0-beta.2", "0.1.0-alpha.10"));
        // 相同
        assert!(!is_newer("0.1.0-rc.6", "0.1.0-rc.6"));
        // 标识符少者旧：rc.6 < rc.6.1
        assert!(is_newer("0.1.0-rc.6.1", "0.1.0-rc.6"));
    }
}
