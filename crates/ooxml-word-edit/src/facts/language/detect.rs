//! Owns: the language a text is written in by its function words (docx-agent's
//! `text_language.detect`, round 49 R94): each language's profile of function words
//! ([`PROFILES`], written for docx-agent's module, no list taken from elsewhere), the
//! spellings each language shows ([`CUES`]) and the judgement ([`detect`]).
//!
//! A profile word in several languages' profiles counts for each a share (`1/n`); a word in
//! none counts [`CUE_WEIGHT`] for the languages whose spellings it shows. The language with
//! the highest score wins; its confidence is how far it leads the next, `(best - second) /
//! best`. A text of fewer than [`MIN_WORDS`] words, or fewer than [`MIN_HITS`] profile words
//! (or under [`MIN_HIT_SHARE`] of its words), is not judged.
//!
//! Must not: read a document.

use super::super::py_letter;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The languages detected, in the order a tie is decided.
pub const LANGUAGES: [&str; 7] = ["en", "fr", "nl", "de", "es", "it", "pt"];

/// Words a text needs to be judged.
pub const MIN_WORDS: usize = 8;
/// Profile words a text needs to be judged.
const MIN_HITS: usize = 3;
/// The share of a text's words its profile words must be at least.
const MIN_HIT_SHARE: f64 = 0.15;
/// What a word's cues count, together (a profile word counts 1).
const CUE_WEIGHT: f64 = 0.5;

/// Each language's function words, in [`LANGUAGES`]' order (lower case; an elided form with
/// its apostrophe).
const PROFILES: [&str; 7] = [
    "the of and to in is are was were be been being it its that this these those with for on at by
     from as an a or not but have has had will would can could should shall which who whom whose what
     when where why how their they them we our us you your he she his her him there than then also
     into about after before all each more most some such only if do does did may might must no so
     very over between through under while both any other since until upon because whether",
    "le la les l' un une des du de d' à et est sont était étaient être été a ont avait avaient au aux
     en dans par pour sur avec sans sous ce cet cette ces qui que qu' dont où il elle ils elles nous
     vous on je se s' son sa ses leur leurs notre nos votre vos ne n' pas plus mais ou donc car si
     aussi très comme tout tous toute toutes entre chez depuis après avant déjà encore même peut
     peuvent doit doivent c' cela ceci lors ainsi lorsque puis alors",
    "de het een en van in is zijn was waren op te dat die dit deze met voor aan bij door om uit naar
     over tot als of maar ook niet geen nog wel al er hij zij ze wij we jullie u ik je hun haar
     wordt worden werd werden heeft hebben had kan kunnen moet moeten zal zullen wat wie waar
     wanneer hoe omdat dus want dan toch zo meer veel alle elke ieder onder tussen sinds na zonder
     daar hier mijn ons onze welke wanneer",
    "der die das den dem des ein eine einen einem einer eines und ist sind war waren sein wird
     werden wurde wurden hat haben hatte in im am an auf aus bei mit nach von vom zu zum zur für
     über unter durch gegen ohne um zwischen seit nicht kein keine auch noch schon nur sehr oder
     aber denn dass wenn weil ob als wie was wer wo er sie es wir ihr ich du man sich ihre unsere
     dieser diese dieses kann können muss müssen soll sollen alle mehr hier dort bereits",
    "el la los las un una unos unas y e o u de del al en con por para sin sobre entre hasta desde
     es son era eran fue ser está están estar ha han había hay que qué quien cual cuando donde como
     lo le les se su sus nuestro nuestra este esta estos estas ese esa pero también muy más no ni
     ya todo todos toda todas cada puede pueden debe deben mientras aunque porque sino nos yo él
     ella ellos",
    "il lo la i gli le l' un uno una un' e ed o di del dello della dei degli delle a al allo alla ai
     agli alle da dal dalla in nel nello nella nei negli nelle con su sul sulla per tra fra è sono
     era erano essere ha hanno aveva che chi cui quale quando dove come non ma anche più molto già
     ancora se si ci questo questa questi queste quello quella ogni tutto tutti suo sua loro nostro
     può possono deve devono dell' all' nell' sull' dall'",
    "o a os as um uma uns umas e ou de do da dos das em no na nos nas ao aos à às com por pelo pela
     para sem sobre entre até desde é são era eram foi ser está estão estar tem têm tinha há que quem
     qual quando onde como não mas também muito mais já ainda se seu sua seus suas este esta isso
     isto esse essa cada todo todos pode podem deve devem nosso nossa ele ela eles você",
];

/// One spelling a cue looks for in a (lower case) word.
#[derive(Clone, Copy)]
enum Cue {
    /// It starts with this.
    Starts(&'static str),
    /// It holds this.
    Holds(&'static str),
    /// It ends with this.
    Ends(&'static str),
    /// It holds one of these characters.
    HoldsOne(&'static str),
    /// It ends with one of these characters.
    EndsOne(&'static str),
}

use Cue::{Ends, EndsOne, Holds, HoldsOne, Starts};

/// Spellings each language has and the others mostly lack, in [`LANGUAGES`]' order: any one
/// found is the language's cue (docx-agent's `CUES` patterns, alternative by alternative).
const CUES: [&[Cue]; 7] = [
    &[
        Starts("th"),
        Starts("wh"),
        Holds("w"),
        Ends("ing"),
        Ends("ould"),
        Ends("ly"),
        Ends("ness"),
        Holds("ough"),
    ],
    &[
        HoldsOne("çœ"),
        HoldsOne("éèêëâîôû"),
        Holds("eau"),
        Ends("aient"),
        Ends("ais"),
        Ends("ait"),
        Ends("ez"),
        Ends("eux"),
        Ends("oir"),
    ],
    &[
        Holds("ij"),
        Holds("oe"),
        Holds("aa"),
        Holds("uu"),
        Holds("sch"),
        Holds("cht"),
        Holds("lijk"),
        Ends("heid"),
        Starts("ge"),
        Starts("ver"),
    ],
    &[
        HoldsOne("äöüß"),
        Holds("sch"),
        Ends("ung"),
        Ends("keit"),
        Ends("heit"),
        Holds("tz"),
        Starts("ge"),
        Starts("ver"),
        Starts("zu"),
        Ends("ich"),
        Holds("cht"),
    ],
    &[
        Holds("ñ"),
        Ends("cion"),
        Ends("ción"),
        Ends("ciones"),
        HoldsOne("áíóú"),
        Ends("ado"),
        Ends("ada"),
        Holds("ll"),
        Ends("os"),
        Ends("mente"),
    ],
    &[
        Ends("zione"),
        Ends("zioni"),
        Holds("gli"),
        Holds("cch"),
        Holds("zz"),
        EndsOne("àèìòù"),
        Holds("tt"),
        Ends("sse"),
        Ends("are"),
        Ends("ere"),
        Ends("ire"),
    ],
    &[
        Ends("ção"),
        Ends("ções"),
        HoldsOne("ãõ"),
        Holds("lh"),
        Holds("nh"),
        HoldsOne("âêô"),
        Ends("mente"),
        Ends("ar"),
        Ends("em"),
    ],
];

impl Cue {
    fn found(self, word: &str) -> bool {
        match self {
            Starts(s) => word.starts_with(s),
            Holds(s) => word.contains(s),
            Ends(s) => word.ends_with(s),
            HoldsOne(set) => word.chars().any(|c| set.contains(c)),
            EndsOne(set) => word.chars().next_back().is_some_and(|c| set.contains(c)),
        }
    }
}

/// Each profile word's weight per language (by index in [`LANGUAGES`]): 1 over the number
/// of profiles it is in.
fn weights() -> &'static HashMap<&'static str, Vec<(usize, f64)>> {
    static WEIGHTS: OnceLock<HashMap<&'static str, Vec<(usize, f64)>>> = OnceLock::new();
    WEIGHTS.get_or_init(|| {
        let sets: Vec<Vec<&str>> = PROFILES.iter().map(|p| p.split_whitespace().collect()).collect();
        let mut out: HashMap<&str, Vec<(usize, f64)>> = HashMap::new();
        for words in &sets {
            for word in words {
                out.entry(word).or_insert_with(|| {
                    let within: Vec<usize> = (0..sets.len())
                        .filter(|k| sets.get(*k).is_some_and(|s| s.contains(word)))
                        .collect();
                    #[allow(clippy::cast_precision_loss)] // at most seven languages
                    let share = 1.0 / within.len() as f64;
                    within.into_iter().map(|k| (k, share)).collect()
                });
            }
        }
        out
    })
}

/// A word's cues, as weights per language (`_cued`).
fn cued(word: &str) -> Vec<(usize, f64)> {
    let languages: Vec<usize> = (0..LANGUAGES.len())
        .filter(|k| CUES.get(*k).is_some_and(|cues| cues.iter().any(|c| c.found(word))))
        .collect();
    #[allow(clippy::cast_precision_loss)] // at most seven languages
    let share = CUE_WEIGHT / languages.len().max(1) as f64;
    languages.into_iter().map(|k| (k, share)).collect()
}

/// What [`detect`] found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detected {
    /// The language (index in [`LANGUAGES`]); `None`: not judged.
    pub language: Option<usize>,
    /// How far it leads the next language, rounded to two decimals.
    pub confidence: f64,
    /// The text's words.
    pub words: usize,
}

/// `text`'s words (`words_of`), lower case: letters (`[^\W\d_]+`), an elision with its
/// apostrophe where a letter follows (`l'`); `’ ʼ` and the backtick are apostrophes.
#[must_use]
pub fn words_of(text: &str) -> Vec<String> {
    let folded: Vec<char> = casefold(&text.replace(['’', 'ʼ', '`'], "'")).chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < folded.len() {
        if !folded.get(k).copied().is_some_and(py_letter) {
            k += 1;
            continue;
        }
        let start = k;
        while folded.get(k).copied().is_some_and(py_letter) {
            k += 1;
        }
        if folded.get(k) == Some(&'\'') && folded.get(k + 1).copied().is_some_and(py_letter) {
            k += 1;
        }
        out.push(folded.get(start..k).unwrap_or_default().iter().collect());
    }
    out
}

/// Python's `str.casefold` for the scripts these languages are written in: lower case, with
/// the sharp s, long s, final sigma and the Latin ligatures folded.
fn casefold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            'ß' | 'ẞ' => out.push_str("ss"),
            'ſ' => out.push('s'),
            'ς' => out.push('σ'),
            'ﬀ' => out.push_str("ff"),
            'ﬁ' => out.push_str("fi"),
            'ﬂ' => out.push_str("fl"),
            'ﬃ' => out.push_str("ffi"),
            'ﬄ' => out.push_str("ffl"),
            'ﬅ' | 'ﬆ' => out.push_str("st"),
            other => out.extend(other.to_lowercase()),
        }
    }
    out
}

/// Python's `round(x, 2)`: the nearest of two decimals, of the exact value, ties to even.
pub(super) fn two_places(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// The language of `text` by its function words.
#[must_use]
pub fn detect(text: &str) -> Detected {
    let words = words_of(text);
    let not_judged = Detected {
        language: None,
        confidence: 0.0,
        words: words.len(),
    };
    if words.len() < MIN_WORDS {
        return not_judged;
    }
    let mut scores = [0.0_f64; LANGUAGES.len()];
    let mut hits = 0_usize;
    let table = weights();
    for word in &words {
        let found = match table.get(word.as_str()) {
            Some(weights) => {
                hits += 1;
                weights.clone()
            }
            None => cued(word),
        };
        for (k, weight) in found {
            if let Some(score) = scores.get_mut(k) {
                *score += weight;
            }
        }
    }
    let total: f64 = scores.iter().sum();
    #[allow(clippy::cast_precision_loss)] // a paragraph's words
    let share = MIN_HIT_SHARE * words.len() as f64;
    #[allow(clippy::cast_precision_loss)]
    let few = (hits as f64) < share;
    if hits < MIN_HITS || few || total <= 0.0 {
        return not_judged;
    }
    let mut ranked: Vec<usize> = (0..LANGUAGES.len()).collect();
    ranked.sort_by(|a, b| {
        let (a, b) = (scores.get(*a).copied(), scores.get(*b).copied());
        b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
    });
    let best = ranked.first().and_then(|k| scores.get(*k)).copied().unwrap_or(0.0);
    let second = ranked.get(1).and_then(|k| scores.get(*k)).copied().unwrap_or(0.0);
    Detected {
        language: ranked.first().copied(),
        confidence: two_places((best - second) / best),
        words: words.len(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn words_keep_an_elision_with_its_apostrophe() {
        assert_eq!(
            words_of("L’homme d'État, 12 x_y Straße"),
            ["l'", "homme", "d'", "état", "x", "y", "strasse"]
        );
        assert_eq!(words_of("don't' end'"), ["don'", "t", "end"]);
    }

    #[test]
    fn function_words_tell_the_language() {
        let found = detect("Le chat est sur la table et il dort dans le soleil du matin.");
        assert_eq!(found.language.map(|k| LANGUAGES[k]), Some("fr"));
        let found = detect("The cat is on the table and it sleeps in the sun of the morning.");
        assert_eq!(found.language.map(|k| LANGUAGES[k]), Some("en"));
        assert_eq!(detect("Too short to judge").language, None);
        let lorem = detect("Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod");
        assert_eq!(lorem.language, None);
    }

    #[test]
    fn two_places_rounds_as_python() {
        assert!((two_places(0.125) - 0.12).abs() < 1e-12);
        assert!((two_places(0.375) - 0.38).abs() < 1e-12);
        assert!((two_places(2.0 / 3.0) - 0.67).abs() < 1e-12);
    }
}
