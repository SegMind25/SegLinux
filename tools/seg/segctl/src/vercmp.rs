//! Package versions and cache file names, as pacman orders and writes them.
//! `vercmp` follows libalpm's `rpmvercmp`, so `seg clean` keeps the same
//! "newest" packages `paccache` would.

use std::cmp::Ordering;

fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Compare two version strings without epoch or release (libalpm's
/// `rpmvercmp`).
fn rpmvercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let at = |s: &[u8], k: usize| s.get(k).copied();
    while at(a, i).is_some() && at(b, j).is_some() {
        let (start_i, start_j) = (i, j);
        while at(a, i).is_some_and(|c| !is_alnum(c)) {
            i += 1;
        }
        while at(b, j).is_some_and(|c| !is_alnum(c)) {
            j += 1;
        }
        if at(a, i).is_none() || at(b, j).is_none() {
            break;
        }
        // Different separator lengths decide it.
        if i - start_i != j - start_j {
            return if i - start_i < j - start_j {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        // A whole numeric or whole alphabetic segment from each side.
        let numeric = at(a, i).is_some_and(|c| c.is_ascii_digit());
        let (mut ei, mut ej) = (i, j);
        if numeric {
            while at(a, ei).is_some_and(|c| c.is_ascii_digit()) {
                ei += 1;
            }
            while at(b, ej).is_some_and(|c| c.is_ascii_digit()) {
                ej += 1;
            }
        } else {
            while at(a, ei).is_some_and(|c| c.is_ascii_alphabetic()) {
                ei += 1;
            }
            while at(b, ej).is_some_and(|c| c.is_ascii_alphabetic()) {
                ej += 1;
            }
        }
        let (mut seg_a, mut seg_b) = (a.get(i..ei).unwrap_or(&[]), b.get(j..ej).unwrap_or(&[]));
        // The other side has a segment of the other kind: numbers win.
        if seg_b.is_empty() {
            return if numeric {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        if numeric {
            while seg_a.first() == Some(&b'0') {
                seg_a = seg_a.get(1..).unwrap_or(&[]);
            }
            while seg_b.first() == Some(&b'0') {
                seg_b = seg_b.get(1..).unwrap_or(&[]);
            }
            // More digits is the bigger number.
            match seg_a.len().cmp(&seg_b.len()) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        match seg_a.cmp(seg_b) {
            Ordering::Equal => {}
            other => return other,
        }
        i = ei;
        j = ej;
    }
    // Everything compared equal; only separators may differ.
    match (at(a, i), at(b, j)) {
        (None, None) => Ordering::Equal,
        // The longer version wins, unless what is left of it is letters
        // (1.5b is older than 1.5).
        (a_rest, b_rest) => {
            let a_empty_b_not_alpha =
                a_rest.is_none() && !b_rest.is_some_and(|c| c.is_ascii_alphabetic());
            if a_empty_b_not_alpha || a_rest.is_some_and(|c| c.is_ascii_alphabetic()) {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
    }
}

/// Split `[epoch:]version[-release]`.
fn split_evr(evr: &str) -> (&str, &str, Option<&str>) {
    let digits = evr.bytes().take_while(u8::is_ascii_digit).count();
    let (epoch, rest) = match evr.get(digits..).and_then(|r| r.strip_prefix(':')) {
        Some(rest) => (if digits == 0 { "0" } else { &evr[..digits] }, rest),
        None => ("0", evr),
    };
    match rest.rsplit_once('-') {
        Some((version, release)) => (epoch, version, Some(release)),
        None => (epoch, rest, None),
    }
}

/// Compare two package versions such as `1:2.4-1` and `2.3.9-3` the way
/// `vercmp` does.
pub fn vercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (e1, v1, r1) = split_evr(a);
    let (e2, v2, r2) = split_evr(b);
    rpmvercmp(e1, e2)
        .then_with(|| rpmvercmp(v1, v2))
        .then_with(|| match (r1, r2) {
            (Some(r1), Some(r2)) => rpmvercmp(r1, r2),
            _ => Ordering::Equal,
        })
}

/// A package file of the pacman cache.
#[derive(Debug, PartialEq, Eq)]
pub struct CacheFile {
    pub name: String,
    /// `[epoch:]pkgver-pkgrel`.
    pub version: String,
    pub arch: String,
    /// A detached signature (`.sig`) of the package file.
    pub is_sig: bool,
}

/// `name-1:2.0-3-x86_64.pkg.tar.zst[.sig]` -> its parts. Other files in the
/// cache (partial downloads, `download-*` folders) give None.
pub fn parse_cache_file(file_name: &str) -> Option<CacheFile> {
    let (stem, is_sig) = match file_name.strip_suffix(".sig") {
        Some(stem) => (stem, true),
        None => (file_name, false),
    };
    let (base, ext) = stem.split_once(".pkg.tar")?;
    // The compressors makepkg knows; a bare ".pkg.tar" is also valid. Anything
    // else (".zst.part", a partial download) is not a finished package.
    if !matches!(
        ext,
        "" | ".zst" | ".xz" | ".gz" | ".bz2" | ".lz4" | ".lz" | ".lrz" | ".lzo" | ".Z"
    ) {
        return None;
    }
    let (rest, arch) = base.rsplit_once('-')?;
    let (rest, release) = rest.rsplit_once('-')?;
    let (name, version) = rest.rsplit_once('-')?;
    if name.is_empty() || version.is_empty() || release.is_empty() || arch.is_empty() {
        return None;
    }
    Some(CacheFile {
        name: name.to_string(),
        version: format!("{version}-{release}"),
        arch: arch.to_string(),
        is_sig,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::*;

    // Cases from pacman's own vercmp test suite (test/util/vercmptest.sh).
    #[test]
    fn matches_pacmans_vercmp() {
        let cases = [
            ("1.5.0", "1.5.0", Equal),
            ("1.5.1", "1.5.0", Greater),
            ("1.5.1", "1.5", Greater),
            ("1.5.0-1", "1.5.0-1", Equal),
            ("1.5.0-1", "1.5.0-2", Less),
            ("1.5.0-1", "1.5.1-1", Less),
            ("1.5.0-2", "1.5.1-1", Less),
            ("1.5-1", "1.5.b2-1", Less),
            ("1.5b-1", "1.5-1", Less),
            ("1.5-1", "1.5b-1", Greater),
            ("1.5.0", "1.5", Greater),
            ("1.0", "1.0.1", Less),
            ("1.0", "1.0a", Greater),
            ("1.0a", "1.0b", Less),
            ("1.0a", "1.0.1", Less),
            ("1.9", "1.10", Less),
            ("1.010", "1.10", Equal),
            ("1.1000", "1.999", Greater),
            ("1:1.0", "2.0", Greater),
            ("1:1.0", "0:2.0", Greater),
            ("2.0", "0:2.0", Equal),
            ("1:1.0-1", "1:1.0-2", Less),
            ("1.0_1", "1.0.1", Equal),
            ("1.0+1", "1.0", Greater),
        ];
        for (a, b, want) in cases {
            assert_eq!(vercmp(a, b), want, "vercmp {a} {b}");
            assert_eq!(vercmp(b, a), want.reverse(), "vercmp {b} {a}");
        }
    }

    #[test]
    fn evr_parts() {
        assert_eq!(split_evr("1:2.0-3"), ("1", "2.0", Some("3")));
        assert_eq!(split_evr("2.0-3"), ("0", "2.0", Some("3")));
        assert_eq!(split_evr("2.0"), ("0", "2.0", None));
        assert_eq!(split_evr(":2.0-3"), ("0", "2.0", Some("3")));
        assert_eq!(split_evr("a:2.0"), ("0", "a:2.0", None));
    }

    #[test]
    fn cache_file_names() {
        let f = parse_cache_file("linux-zen-headers-6.12.4.zen1-1-x86_64.pkg.tar.zst").unwrap();
        assert_eq!(f.name, "linux-zen-headers");
        assert_eq!(f.version, "6.12.4.zen1-1");
        assert_eq!(f.arch, "x86_64");
        assert!(!f.is_sig);

        let f = parse_cache_file("qt6-base-1:6.8.1-2-x86_64.pkg.tar.xz.sig").unwrap();
        assert_eq!(f.name, "qt6-base");
        assert_eq!(f.version, "1:6.8.1-2");
        assert!(f.is_sig);

        let f = parse_cache_file("filesystem-2024.11-1-any.pkg.tar.zst").unwrap();
        assert_eq!((f.name.as_str(), f.arch.as_str()), ("filesystem", "any"));

        assert!(parse_cache_file("download-abc123").is_none());
        assert!(parse_cache_file("foo-1.0-1-x86_64.pkg.tar.zst.part").is_none());
        assert!(parse_cache_file("foo.pkg.tar.zst").is_none());
        assert!(parse_cache_file("-1.0-1-x86_64.pkg.tar.zst").is_none());
        assert!(parse_cache_file("readme.txt").is_none());
    }
}
