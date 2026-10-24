//! Kana → romaji (modified Hepburn) and mora counting for lyric lines.
//!
//! Only kana can be converted: kanji readings need a dictionary, which is
//! not bundled, so lines with kanji keep the kanji in the output and report
//! `has_kanji` so the UI can ask for a reading.

/// Result of converting one lyric line.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Romanized {
    pub romaji: String,
    /// Number of morae (拍) when the text is all kana, else `None`.
    pub morae: Option<u32>,
    pub has_kanji: bool,
}

fn to_hiragana(c: char) -> char {
    match c {
        'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
        _ => c,
    }
}

fn base(c: char) -> Option<&'static str> {
    Some(match c {
        'あ' => "a", 'い' => "i", 'う' => "u", 'え' => "e", 'お' => "o",
        'か' => "ka", 'き' => "ki", 'く' => "ku", 'け' => "ke", 'こ' => "ko",
        'が' => "ga", 'ぎ' => "gi", 'ぐ' => "gu", 'げ' => "ge", 'ご' => "go",
        'さ' => "sa", 'し' => "shi", 'す' => "su", 'せ' => "se", 'そ' => "so",
        'ざ' => "za", 'じ' => "ji", 'ず' => "zu", 'ぜ' => "ze", 'ぞ' => "zo",
        'た' => "ta", 'ち' => "chi", 'つ' => "tsu", 'て' => "te", 'と' => "to",
        'だ' => "da", 'ぢ' => "ji", 'づ' => "zu", 'で' => "de", 'ど' => "do",
        'な' => "na", 'に' => "ni", 'ぬ' => "nu", 'ね' => "ne", 'の' => "no",
        'は' => "ha", 'ひ' => "hi", 'ふ' => "fu", 'へ' => "he", 'ほ' => "ho",
        'ば' => "ba", 'び' => "bi", 'ぶ' => "bu", 'べ' => "be", 'ぼ' => "bo",
        'ぱ' => "pa", 'ぴ' => "pi", 'ぷ' => "pu", 'ぺ' => "pe", 'ぽ' => "po",
        'ま' => "ma", 'み' => "mi", 'む' => "mu", 'め' => "me", 'も' => "mo",
        'や' => "ya", 'ゆ' => "yu", 'よ' => "yo",
        'ら' => "ra", 'り' => "ri", 'る' => "ru", 'れ' => "re", 'ろ' => "ro",
        'わ' => "wa", 'ゐ' => "i", 'ゑ' => "e", 'を' => "o", 'ゔ' => "vu",
        'ぁ' => "a", 'ぃ' => "i", 'ぅ' => "u", 'ぇ' => "e", 'ぉ' => "o",
        'ゃ' => "ya", 'ゅ' => "yu", 'ょ' => "yo", 'ゎ' => "wa",
        _ => return None,
    })
}

fn is_small_y(c: char) -> bool {
    matches!(c, 'ゃ' | 'ゅ' | 'ょ')
}

fn is_small_vowel(c: char) -> bool {
    matches!(c, 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ')
}

/// Syllable formed by `c` followed by a small kana, if it combines.
fn digraph(c: char, small: char) -> Option<String> {
    let v = base(small)?;
    if is_small_y(small) {
        let y = &v[1..]; // a / u / o
        return Some(match c {
            'し' => format!("sh{y}"),
            'ち' => format!("ch{y}"),
            'じ' | 'ぢ' => format!("j{y}"),
            'き' | 'ぎ' | 'に' | 'ひ' | 'び' | 'ぴ' | 'み' | 'り' => {
                let b = base(c)?;
                format!("{}y{y}", &b[..b.len() - 1])
            }
            _ => return None,
        });
    }
    if is_small_vowel(small) {
        return Some(match (c, small) {
            ('ふ', _) => format!("f{v}"),
            ('て', 'ぃ') => "ti".into(),
            ('で', 'ぃ') => "di".into(),
            ('と', 'ぅ') => "tu".into(),
            ('ど', 'ぅ') => "du".into(),
            ('う', 'ぃ') => "wi".into(),
            ('う', 'ぇ') => "we".into(),
            ('う', 'ぉ') => "wo".into(),
            ('ゔ', _) => format!("v{v}"),
            ('し', 'ぇ') => "she".into(),
            ('じ', 'ぇ') => "je".into(),
            ('ち', 'ぇ') => "che".into(),
            ('つ', _) => format!("ts{v}"),
            _ => return None,
        });
    }
    None
}

fn is_kanji(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF) || c == '々'
}

fn punctuation(c: char) -> Option<&'static str> {
    Some(match c {
        '、' | '，' => ", ",
        '。' | '．' => ". ",
        '！' => "! ",
        '？' => "? ",
        '「' | '」' | '『' | '』' => "\"",
        '　' => " ",
        '・' => " ",
        '～' | '〜' => "~",
        _ => return None,
    })
}

/// Convert kana (hiragana or katakana) to modified-Hepburn romaji, counting
/// morae. Other characters are kept as they are.
pub fn romanize(text: &str) -> Romanized {
    let chars: Vec<char> = text.chars().map(to_hiragana).collect();
    let mut out = String::new();
    let mut morae = 0u32;
    let mut has_kanji = false;
    let mut other = false;
    let mut geminate = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == 'っ' {
            geminate = true;
            morae += 1;
            i += 1;
            continue;
        }
        if c == 'ん' {
            let next_vowel_or_y = chars.get(i + 1).and_then(|n| base(*n)).map(|s| s.starts_with(['a', 'i', 'u', 'e', 'o', 'y'])).unwrap_or(false);
            out.push_str(if next_vowel_or_y { "n'" } else { "n" });
            morae += 1;
            i += 1;
            continue;
        }
        if c == 'ー' {
            if let Some(v) = out.chars().rev().find(|ch| "aiueo".contains(*ch)) {
                out.push(v);
            }
            morae += 1;
            i += 1;
            continue;
        }
        if let Some(b) = base(c) {
            let (syl, used) = match chars.get(i + 1) {
                Some(&s) if is_small_y(s) || is_small_vowel(s) => match digraph(c, s) {
                    Some(d) => (d, 2),
                    None => (b.to_string(), 1),
                },
                _ => (b.to_string(), 1),
            };
            if geminate {
                if syl.starts_with("ch") {
                    out.push('t');
                } else if let Some(f) = syl.chars().next().filter(|f| !"aiueo".contains(*f)) {
                    out.push(f);
                }
                geminate = false;
            }
            out.push_str(&syl);
            morae += 1;
            i += used;
            continue;
        }
        if geminate {
            geminate = false;
        }
        if let Some(p) = punctuation(c) {
            out.push_str(p);
        } else if c.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            if is_kanji(c) {
                has_kanji = true;
            } else if !c.is_ascii_punctuation() {
                other = true;
            }
            out.push(c);
        }
        i += 1;
    }
    let romaji = out.split_whitespace().collect::<Vec<_>>().join(" ");
    Romanized { romaji, morae: (!has_kanji && !other && morae > 0).then_some(morae), has_kanji }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        romanize(s).romaji
    }

    #[test]
    fn basic_hiragana_and_katakana() {
        assert_eq!(r("あいうえお"), "aiueo");
        assert_eq!(r("さくら"), "sakura");
        assert_eq!(r("サクラ"), "sakura");
        assert_eq!(r("しちつふ"), "shichitsufu");
    }

    #[test]
    fn digraphs_geminates_and_n() {
        assert_eq!(r("きょう"), "kyou");
        assert_eq!(r("しゃしん"), "shashin");
        assert_eq!(r("じゃあ"), "jaa");
        assert_eq!(r("がっこう"), "gakkou");
        assert_eq!(r("まっちゃ"), "matcha");
        assert_eq!(r("きっぷ"), "kippu");
        assert_eq!(r("こんや"), "kon'ya");
        assert_eq!(r("せんえん"), "sen'en");
        assert_eq!(r("しんぶん"), "shinbun");
    }

    #[test]
    fn foreign_sounds_and_long_vowels() {
        assert_eq!(r("ファン"), "fan");
        assert_eq!(r("パーティー"), "paatii");
        assert_eq!(r("ヴァイオリン"), "vaiorin");
        assert_eq!(r("ウェブ"), "webu");
    }

    #[test]
    fn mora_counting() {
        assert_eq!(romanize("がっこう").morae, Some(4));
        assert_eq!(romanize("きょう").morae, Some(2));
        assert_eq!(romanize("しんぶん").morae, Some(4));
        assert_eq!(romanize("パーティー").morae, Some(4));
        assert_eq!(romanize("さくら さくら").morae, Some(6));
    }

    #[test]
    fn kanji_and_punctuation() {
        let k = romanize("夜に駆ける");
        assert!(k.has_kanji);
        assert_eq!(k.morae, None);
        assert_eq!(k.romaji, "夜ni駆keru");
        assert_eq!(r("はい、そうです。"), "hai, soudesu.");
        assert_eq!(r("「うた」"), "\"uta\"");
    }
}
