//! The arithmetic behind the charts (`components/huginn/scale.ts`), and the
//! number format every component shares.

use crate::i18n::Locale;

#[derive(Debug, Clone, PartialEq)]
pub struct Domain {
    pub min: f64,
    pub max: f64,
    pub ticks: Vec<f64>,
}

/// JavaScript's `Number(x.toPrecision(12))`.
fn precision12(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{x:.11e}").parse().unwrap_or(x)
}

/// Round tick marks that cover the values; `include_zero` for bars and areas.
pub fn nice_domain(values: &[Option<f64>], include_zero: bool) -> Domain {
    let known: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    let count = 4.0;
    let mut low = known.iter().copied().fold(f64::INFINITY, f64::min);
    let mut high = known.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if known.is_empty() {
        low = 0.0;
        high = 1.0;
    }
    if include_zero {
        low = low.min(0.0);
        high = high.max(0.0);
    }
    if low == high {
        let widen = |v: f64| {
            if v.abs() * 0.1 == 0.0 {
                1.0
            } else {
                v.abs() * 0.1
            }
        };
        let (l, h) = (low, high);
        low = l - widen(l);
        high = h + widen(h);
    }
    let rough = (high - low) / count;
    let magnitude = 10f64.powf(rough.log10().floor());
    let residual = rough / magnitude;
    let step = (if residual >= 5.0 {
        10.0
    } else if residual >= 2.0 {
        5.0
    } else if residual >= 1.5 {
        2.0
    } else {
        1.0
    }) * magnitude;
    let first = (low / step).floor() * step;
    let last = (high / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut i = 0.0;
    while first + i * step <= last + step / 2.0 && ticks.len() < 1000 {
        ticks.push(precision12(first + i * step));
        i += 1.0;
    }
    Domain {
        min: first,
        max: last,
        ticks,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

/// Positive values pile up from zero and negative ones down from it.
pub fn stack(values: &[Option<f64>]) -> Vec<Segment> {
    let (mut up, mut down) = (0.0, 0.0);
    values
        .iter()
        .map(|v| {
            let amount = v.unwrap_or(0.0);
            if amount >= 0.0 {
                let start = up;
                up += amount;
                Segment { start, end: up }
            } else {
                let start = down;
                down += amount;
                Segment { start, end: down }
            }
        })
        .collect()
}

/// Which labels fit, evenly spread, always keeping the first.
pub fn thinned(count: usize, room: f32, label: f32) -> impl Fn(usize) -> bool {
    let fit = ((room / label).floor() as i64).max(1) as usize;
    let every = count.div_ceil(fit).max(1);
    move |i| i % every == 0
}

/// `Intl.NumberFormat(locale, {maximumFractionDigits: 2})`, plus the unit.
pub fn format_number(
    value: Option<f64>,
    unit: Option<&str>,
    locale: Locale,
    no_data: &str,
) -> String {
    let Some(v) = value else {
        return no_data.to_string();
    };
    let number = format_plain(v, locale);
    match unit {
        Some(u) if !u.is_empty() => format!("{number} {u}"),
        _ => number,
    }
}

/// The number alone, grouped and rounded half away from zero to two places
/// from its shortest decimal form, as ICU does.
pub fn format_plain(v: f64, locale: Locale) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let negative = v.is_sign_negative();
    let digits = format!("{}", v.abs());
    let (int_part, frac_part) = digits.split_once('.').unwrap_or((&digits, ""));
    let mut int_digits: Vec<u8> = int_part.bytes().map(|b| b - b'0').collect();
    let mut frac: Vec<u8> = frac_part.bytes().map(|b| b - b'0').collect();
    if frac.len() > 2 {
        let up = frac[2] >= 5;
        frac.truncate(2);
        if up {
            let mut carry = true;
            for d in frac.iter_mut().rev() {
                if !carry {
                    break;
                }
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    carry = false;
                }
            }
            if carry {
                for d in int_digits.iter_mut().rev() {
                    if !carry {
                        break;
                    }
                    if *d == 9 {
                        *d = 0;
                    } else {
                        *d += 1;
                        carry = false;
                    }
                }
                if carry {
                    int_digits.insert(0, 1);
                }
            }
        }
    }
    while frac.last() == Some(&0) {
        frac.pop();
    }
    let (group, decimal, min_group) = match locale {
        Locale::En => (',', '.', 4),
        // Spanish leaves four-digit numbers ungrouped.
        Locale::Es => ('.', ',', 5),
    };
    let int_text: String = int_digits.iter().map(|d| char::from(b'0' + d)).collect();
    let mut grouped = String::new();
    if int_text.len() >= min_group {
        for (i, c) in int_text.chars().enumerate() {
            if i > 0 && (int_text.len() - i).is_multiple_of(3) {
                grouped.push(group);
            }
            grouped.push(c);
        }
    } else {
        grouped = int_text;
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if !frac.is_empty() {
        out.push(decimal);
        out.extend(frac.iter().map(|d| char::from(b'0' + d)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(v: &[f64]) -> Vec<Option<f64>> {
        v.iter().copied().map(Some).collect()
    }

    #[test]
    fn domains_are_round_and_cover_the_values() {
        let d = nice_domain(&some(&[3.0, 47.0]), true);
        assert_eq!((d.min, d.max), (0.0, 50.0));
        assert_eq!(d.ticks, vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0]);
        let d = nice_domain(&some(&[0.1, 0.3]), false);
        assert_eq!(d.ticks, vec![0.1, 0.15, 0.2, 0.25, 0.3]);
        let d = nice_domain(&[None, None], true);
        assert_eq!((d.min, d.max), (0.0, 1.0));
        let d = nice_domain(&some(&[5.0]), false);
        assert!(d.min <= 4.5 && d.max >= 5.5);
        let d = nice_domain(&some(&[-12.0, 30.0]), true);
        assert_eq!((d.min, d.max), (-20.0, 30.0));
    }

    #[test]
    fn stacks_pile_each_sign_apart() {
        let s = stack(&[Some(2.0), Some(-1.0), None, Some(3.0), Some(-4.0)]);
        assert_eq!(
            s[0],
            Segment {
                start: 0.0,
                end: 2.0
            }
        );
        assert_eq!(
            s[1],
            Segment {
                start: 0.0,
                end: -1.0
            }
        );
        assert_eq!(
            s[2],
            Segment {
                start: 2.0,
                end: 2.0
            }
        );
        assert_eq!(
            s[3],
            Segment {
                start: 2.0,
                end: 5.0
            }
        );
        assert_eq!(
            s[4],
            Segment {
                start: -1.0,
                end: -5.0
            }
        );
    }

    #[test]
    fn labels_thin_out_from_the_first() {
        let keep = thinned(10, 200.0, 64.0);
        let kept: Vec<usize> = (0..10).filter(|i| keep(*i)).collect();
        assert_eq!(kept, vec![0, 4, 8]);
        let keep = thinned(3, 600.0, 64.0);
        assert!((0..3).all(keep));
    }

    #[test]
    fn numbers_format_like_intl() {
        assert_eq!(format_plain(1234.5, Locale::En), "1,234.5");
        assert_eq!(format_plain(1234.5, Locale::Es), "1234,5");
        assert_eq!(format_plain(12345.678, Locale::Es), "12.345,68");
        assert_eq!(format_plain(1.005, Locale::En), "1.01");
        assert_eq!(format_plain(0.125, Locale::En), "0.13");
        assert_eq!(format_plain(-2.0, Locale::En), "-2");
        assert_eq!(format_plain(9.999, Locale::En), "10");
        assert_eq!(format_plain(999999.999, Locale::En), "1,000,000");
        assert_eq!(
            format_number(Some(3.0), Some("ms"), Locale::En, "no data"),
            "3 ms"
        );
        assert_eq!(
            format_number(None, Some("ms"), Locale::En, "no data"),
            "no data"
        );
    }
}
