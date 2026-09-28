use anyhow::Result;
use camino::Utf8PathBuf;
use clap::Parser;

use crate::model::{FrequencyModel, Sym};

mod encode;
mod model;

#[derive(Parser)]
struct Args {
    #[clap(short = 'o', long)]
    max_order: u32,
    corpus: Utf8PathBuf,
}

fn parse_freqs<'a>(input: impl Iterator<Item = &'a str>, max_order: usize) -> FrequencyModel {
    let mut freqs = FrequencyModel::new();

    for line in input {
        // pad input on left a zero byte and terminate with a zero byte (EOF)
        let padded = [0u8; 1]
            .into_iter()
            .chain(line.bytes())
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();

        // record the frequency of each symbol for all prefixes of length up to `max_order`
        for pos in 0..padded.len() {
            let sym = Sym(padded[pos]);
            for prefix_len in 0..=max_order.min(pos) {
                if pos == 0 && prefix_len == 0 {
                    // don't record the leading zero byte as a symbol in the empty context
                    continue;
                }

                let prefix = padded[pos - prefix_len..pos].to_vec();

                freqs.update(&prefix, sym);
            }
        }
    }

    // input.par_bridge().map();

    freqs.finish();

    freqs
}

fn main() -> Result<()> {
    let args = Args::parse();

    let input = std::fs::read_to_string(args.corpus)?;
    let freqs = parse_freqs(input.lines().map(str::trim), args.max_order as usize);

    // println!("{:?}", freqs);

    let s = "\0https://dictionary.cambridge.org/dictionary/english/please\0";
    let mut bits = 0.0;
    // *do* encode the terminating zero byte, but not the leading one
    // (only used for indicating the start of the string in the model)
    for i in 1..s.len() {
        let prefix = &s.as_bytes()[i.saturating_sub(args.max_order as usize)..i];
        let sym = Sym(s.as_bytes()[i]);

        let evs = crate::encode::events_for_symbol(&freqs, prefix, args.max_order as usize, sym);
        println!(
            "prefix: {:?}, sym: {:?}, events: {:?}",
            bstr::BStr::new(prefix),
            sym.0 as char,
            evs
        );

        for ev in evs {
            bits += ev.entropy();
        }
    }

    println!(
        "est. bits to encode: {:.2} ({:.2} bytes, {:.2}% of original)",
        bits,
        bits / 8.0,
        bits / (s.len() * 8) as f64 * 100.0
    );

    Ok(())
}
