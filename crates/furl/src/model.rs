use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};

use ordered_float::NotNan;
use radix_trie::{Trie, TrieCommon as _};

/// A "symbol" in the input stream. Represented as bytes.
// TODO: is there a better representation, given that we only care about valid URIs?
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Sym(pub u8);

impl Sym {
    const MAX: u8 = 255;
}

impl From<Sym> for usize {
    fn from(s: Sym) -> Self {
        s.0 as usize
    }
}

/// Frequency distribution for a given prefix, for a given order.
#[derive(Clone, Default)]
pub(crate) struct FreqCtx {
    // sparse representation of counts for each symbol (byte) that follows the prefix
    counts: HashMap<u8, u32>,
}

impl std::ops::AddAssign for FreqCtx {
    fn add_assign(&mut self, rhs: Self) {
        for (b, c) in rhs.counts {
            self.counts.entry(b).and_modify(|x| *x += c).or_insert(c);
        }
    }
}

impl FreqCtx {
    pub fn from_sym(sym: Sym) -> Self {
        Self {
            counts: HashMap::from_iter([(sym.0, 1)]),
        }
    }

    pub fn update(&mut self, sym: Sym) {
        self.counts
            .entry(sym.0)
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    pub fn distinct(&self) -> u8 {
        self.counts.len() as u8
    }

    pub fn total(&self) -> u32 {
        self.counts.values().sum()
    }

    pub fn counts(&self) -> &HashMap<u8, u32> {
        &self.counts
    }
}

impl std::fmt::Debug for FreqCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut exists = self
            .counts
            .iter()
            .filter(|&(_, c)| *c > 0)
            .map(|(s, c)| (char::from(*s), *c as f64 / self.total() as f64))
            .collect::<Vec<_>>();

        exists.sort_by_key(|(_, c)| Reverse(NotNan::new(*c).expect("not nan")));

        let vals = exists
            .iter()
            .map(|(s, c)| {
                format!(
                    "{} {:.2}",
                    if *s == '\0' {
                        String::from("\\0")
                    } else {
                        s.to_string()
                    },
                    c * 100.
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        write!(
            f,
            " {{ t {}, d {}, c [{vals}] }}",
            self.total(),
            self.distinct()
        )?;

        Ok(())
    }
}

pub(crate) struct FrequencyModel {
    // for each prefix, the frequency distribution of symbols that follow it
    freqs: Trie<Vec<u8>, FreqCtx>,
}

impl FrequencyModel {
    pub fn new() -> Self {
        FrequencyModel { freqs: Trie::new() }
    }

    pub fn update(&mut self, prefix: &[u8], sym: Sym) {
        self.freqs.map_with_default(
            prefix.to_vec(),
            |ctx: &mut FreqCtx| ctx.update(sym),
            FreqCtx::from_sym(sym),
        );
    }

    pub fn finish(&mut self) {
        // apply a baseline smoothing to order 0 (empty prefix) for all byte values to guarantee termination
        let order0 = self.freqs.get_mut(&vec![]).expect("corpus is empty");
        for b in 0..=Sym::MAX {
            order0.update(Sym(b));
        }
    }

    pub fn get(&self, prefix: &[u8]) -> Option<&FreqCtx> {
        self.freqs.get(&prefix.to_vec())
    }

    /// Get the coding parameters of a symbol in the context of a given prefix.
    ///
    /// Returns None if the prefix has no frequency context, or if all
    /// symbols in the context are excluded. Returns Some((cum, freq, total)) where:
    /// - cum: cumulative frequency of all symbols less than the given symbol
    /// - freq: frequency of the given symbol
    /// - total: total frequency of all symbols in the context, plus the number of distinct
    ///   symbols (for escape events)
    ///
    /// If the symbol is not present in the context, returns
    /// Some((total, distinct, total + distinct)) for an escape event.
    pub fn get_event(
        &self,
        prefix: &[u8],
        sym: Sym,
        exclude: &[bool; Sym::MAX as usize + 1],
    ) -> Option<(u32, u32, u32, bool)> {
        let ctx = self.freqs.get(&prefix.to_vec())?;

        let active = ctx
            .counts
            .iter()
            .filter(|&(b, _)| !exclude[*b as usize])
            .map(|(b, c)| (*b, *c))
            .collect::<BTreeMap<_, _>>();

        if active.is_empty() {
            return None;
        }

        let total: u32 = active.values().sum();
        let distinct = active.len() as u32;
        let denom = total + distinct;

        let cum = active
            .iter()
            .take_while(|&(b, _)| *b != sym.0)
            .map(|(_, c)| c)
            .sum();

        if let Some(freq) = active.get(&sym.0) {
            Some((cum, *freq, denom, false))
        } else {
            // escape code, independent of symbol
            Some((total, distinct, denom, true))
        }
    }
}

impl std::fmt::Debug for FrequencyModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (prefix, ctx) in self.freqs.iter() {
            writeln!(f, "prefix: {:?}, ctx: {:?}", bstr::BStr::new(&prefix), ctx)?;
        }
        Ok(())
    }
}
