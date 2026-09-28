use std::cmp::Reverse;

use anyhow::Result;
use camino::Utf8PathBuf;
use clap::Parser;
use litemap::LiteMap;
use ordered_float::NotNan;
use radix_trie::{Trie, TrieCommon as _};

#[derive(Parser)]
struct Args {
    #[clap(short = 'o', long)]
    max_order: u32,
    corpus: Utf8PathBuf,
}

/// A "symbol" in the input stream. Represented as bytes.
// TODO: is there a better representation, given that we only care about valid URIs?
#[repr(transparent)]
#[derive(Copy, Clone)]
struct Sym(pub u8);

impl From<Sym> for usize {
    fn from(s: Sym) -> Self {
        s.0 as usize
    }
}

/// Frequency distribution for a given prefix, for a given order.
#[derive(Clone)]
struct FreqCtx {
    // sparse representation of counts for each symbol (byte) that follows the prefix
    counts: LiteMap<u8, u32>,
}

impl FreqCtx {
    fn new() -> Self {
        FreqCtx {
            counts: LiteMap::new(),
        }
    }

    fn from_sym(sym: Sym) -> Self {
        Self {
            counts: LiteMap::from_iter([(sym.0, 1)]),
        }
    }

    fn update(&mut self, sym: Sym) {
        self.counts
            .entry(sym.0)
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    fn distinct(&self) -> u8 {
        self.counts.len() as u8
    }

    fn total(&self) -> u32 {
        self.counts.values().sum()
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

fn parse_freqs<'a>(
    input: impl Iterator<Item = &'a str>,
    max_order: usize,
) -> Trie<Vec<u8>, FreqCtx> {
    let mut trie = Trie::new();

    for line in input {
        // pad input on left a zero byte and terminate with a zero byte (EOF)
        let padded = [0u8; 1]
            .into_iter()
            .chain(line.bytes())
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();

        for window in padded.windows(max_order + 1) {
            // look at each prefix of the window, and update the frequency context for the next symbol
            for prefix_len in 0..=max_order {
                // the symbol is the last byte of the window, and the prefix is the preceding `prefix_len` bytes
                // (for order 0, we simply report the frequency of the symbol itself, with an empty prefix)
                let sym = Sym(window[max_order]);
                let prefix = window[max_order - prefix_len..max_order].to_vec();

                trie.map_with_default(
                    prefix,
                    |ctx: &mut FreqCtx| ctx.update(sym),
                    FreqCtx::from_sym(sym),
                );
            }
        }
    }

    // input.par_bridge().map();

    trie
}

fn main() -> Result<()> {
    let args = Args::parse();

    let input = std::fs::read_to_string(args.corpus)?;
    let trie = parse_freqs(input.lines().map(str::trim), args.max_order as usize);

    // println!("{:?}", trie);
    for (prefix, ctx) in trie.iter() {
        println!("prefix: {:?}, ctx: {:?}", bstr::BStr::new(&prefix), ctx);
    }

    Ok(())
}
