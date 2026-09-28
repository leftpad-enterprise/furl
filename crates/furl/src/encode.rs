use crate::Sym;
use crate::model::FrequencyModel;

/// A single event in the encoding process.
pub(crate) struct Event {
    cum: u32,
    freq: u32,
    total: u32,
}

impl Event {
    pub fn entropy(&self) -> f64 {
        -((self.freq as f64) / (self.total as f64)).log2()
    }
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rel_cum = self.cum as f64 / self.total as f64;
        let rel_freq = self.freq as f64 / self.total as f64;

        write!(
            f,
            "Event {{ cum {:.4}, freq {:.4}, {:.2} bits }}",
            rel_cum,
            rel_freq,
            self.entropy()
        )
    }
}

/// Given a frequency distribution for each context, and a symbol to encode, return the sequence
/// of events that would be emitted by a PPM-style encoder for that symbol, backing off to shorter
/// contexts as necessary.
///
/// `freqs` is required to contain a smoothed distribution for the empty context (order 0) so that
/// the function can always terminate with a valid event.
pub(crate) fn events_for_symbol(
    freqs: &FrequencyModel,
    context: &[u8], // preceding bytes, most recent last
    max_order: usize,
    sym: Sym,
) -> Vec<Event> {
    let mut events = Vec::new();
    let mut excluded = [false; 256];

    for order in (0..=max_order.min(context.len())).rev() {
        let prefix = &context[context.len() - order..];

        let Some((cum, freq, denom, is_escape)) = freqs.get_event(prefix, sym, &excluded) else {
            // no distribution for this context or no active symbols, back off to a shorter context
            continue;
        };

        events.push(Event {
            cum,
            freq,
            total: denom,
        });

        if !is_escape {
            // we have found the symbol in this context, so we can stop
            eprintln!(
                "emitting at order {order} (prefix {:?}) for {:?}",
                bstr::BStr::new(prefix),
                sym.0
            );
            return events;
        } else {
            // exclude all symbols seen in this context from consideration in shorter contexts
            for b in freqs.get(prefix).unwrap().counts().keys() {
                excluded[*b as usize] = true;
            }
        }
    }

    panic!(
        "no distribution found for symbol {:?} in any context",
        sym.0
    );
}
