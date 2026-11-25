//! Charts drawn as SVG on the server: they need no script, so no exception to
//! the Content-Security-Policy. Every bar names its value in a
//! tooltip (`<title>`), as each line's end marker does, every chart of several
//! series has a legend, and every chart shows its numbers as a table too.
//! Series colors are the `--series-*` tokens, which the light and dark themes
//! each set; the second series is dashed too, for whoever can't tell them
//! apart. A drawing scrolls sideways in its frame on a narrow screen, so the
//! frame takes the keyboard's focus; the drawing inside names the chart.

use std::fmt::Write as _;
use topcoat::{
    Result,
    view::{View, component, view},
};

/// One mark: its label, on the axis, in its tooltip and in the table, and its
/// value.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    /// Such as `14:00`.
    pub label: String,
    /// Its value.
    pub value: f64,
}

/// What a page of charts shows: why it has none, or its charts' rows.
#[derive(Clone, Debug, PartialEq)]
pub enum Charted<T> {
    /// Auth reads no analytics back.
    Off,
    /// The analytics database didn't answer.
    Unavailable,
    /// What it counted.
    Shown(T),
}

/// One bar of a bar chart: its label, its length, and what it says, at its tip,
/// in its tooltip and in the table, such as `1,284` or `25+`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// Such as `Sign-in page`.
    pub label: String,
    /// Its length, zero for none.
    pub value: f64,
    /// What it says.
    pub shown: String,
}

impl Bar {
    /// A bar of `value`, saying it.
    #[must_use]
    pub fn of(label: impl Into<String>, value: f64) -> Self {
        Self {
            label: label.into(),
            value,
            shown: compact(value),
        }
    }
}

/// One series of a line chart: its name and its points, one per label.
#[derive(Clone, Debug)]
pub struct Series<'a> {
    /// Its name, in the legend and the table.
    pub name: &'a str,
    /// Its values, one per point of the chart's labels.
    pub values: Vec<f64>,
}

/// The drawing's width, in its own units; it scales to its card, which is
/// about as wide on a laptop, so its text is about as large as the page's.
const WIDTH: f64 = 880.0;
/// The drawing's height.
const HEIGHT: f64 = 240.0;
/// Room left of the plot for the value axis's labels.
const LEFT: f64 = 44.0;
/// Room below the plot for the labels.
const BOTTOM: f64 = 26.0;
/// Room above the plot, so its top label isn't cut.
const TOP: f64 = 8.0;
/// Room right of the plot, so an end marker isn't cut.
const RIGHT: f64 = 8.0;
/// The radius of a bar's data end.
const ROUND: f64 = 4.0;

/// `count` as a coordinate: charts hold far fewer marks than lose precision.
fn float(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The value axis: round steps from zero past `max`, up to five of them.
#[derive(Clone, Debug, PartialEq)]
struct Scale {
    /// Every tick, from zero.
    ticks: Vec<f64>,
}

impl Scale {
    /// The scale that holds `max`. Counts are whole, so a step under 10 is too.
    fn of(max: f64) -> Self {
        let max = if max > 0.0 { max } else { 1.0 };
        let rough = max / 4.0;
        let magnitude = 10_f64.powf(rough.log10().floor());
        let factors: &[f64] = if magnitude >= 10.0 {
            &[1.0, 2.0, 2.5, 5.0, 10.0]
        } else {
            &[1.0, 2.0, 5.0, 10.0]
        };
        let step = factors
            .iter()
            .map(|factor| factor * magnitude)
            .find(|step| *step >= rough)
            .unwrap_or(magnitude * 10.0)
            .max(1.0);
        let mut ticks = vec![0.0];
        while ticks.last().is_some_and(|tick| *tick < max) {
            ticks.push(step * float(ticks.len()));
        }
        Self { ticks }
    }

    /// The top tick.
    fn top(&self) -> f64 {
        self.ticks.last().copied().unwrap_or(1.0)
    }

    /// Where `value` sits, from the plot's top at [`TOP`] to its base.
    fn y(&self, value: f64) -> f64 {
        let base = HEIGHT - BOTTOM;
        base - (value / self.top()) * (base - TOP)
    }
}

/// `value` as an axis or a tip writes it: `1,284`, `12.9K`, `4.2M`.
#[must_use]
pub fn compact(value: f64) -> String {
    let rounded = |value: f64, unit: &str| {
        let text = format!("{value:.1}");
        format!("{}{unit}", text.trim_end_matches(".0"))
    };
    match value.abs() {
        v if v >= 1e6 => rounded(value / 1e6, "M"),
        v if v >= 1e4 => rounded(value / 1e3, "K"),
        v if v.fract() == 0.0 => {
            let digits = format!("{value:.0}");
            let mut grouped = String::new();
            for (i, digit) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i) % 3 == 0 && digit.is_ascii_digit() {
                    grouped.push(',');
                }
                grouped.push(digit);
            }
            grouped
        }
        _ => format!("{value:.1}"),
    }
}

/// Whether the `i`th of `count` labels shows, every `every`th counting back
/// from the last, which always shows, so the newest is always named and no
/// two crowd each other.
const fn labelled(i: usize, count: usize, every: usize) -> bool {
    (count - 1 - i).is_multiple_of(if every == 0 { 1 } else { every })
}

/// The value axis's gridlines and labels.
#[component]
async fn value_axis<'a>(
    /// The axis to draw.
    scale: &'a Scale,
) -> Result<impl View> {
    Ok(view! {
        for tick in &scale.ticks {
            let y = format!("{:.1}", scale.y(*tick));
            <line class="grid" x1=(format!("{LEFT}")) x2=(format!("{}", WIDTH - RIGHT)) y1=(y.clone()) y2=(y.clone())></line>
            <text class="axis" x=(format!("{}", LEFT - 8.0)) y=(y) text-anchor="end" dominant-baseline="middle">(compact(*tick))</text>
        }
    })
}

/// One series of a line chart, drawn.
struct Line<'a> {
    /// Its color's class, `s1` for the first series.
    class: String,
    /// The path through its points.
    path: String,
    /// Where its end marker sits.
    end: (f64, f64),
    /// What its end marker's tooltip says.
    tip: String,
    /// Its name, in the legend.
    name: &'a str,
}

/// A line per series over `labels`, labelling every `every`th on the axis,
/// with a legend, and a marker at each line's end.
#[component]
pub async fn line_chart<'a>(
    /// What the chart shows, for screen readers and the table.
    caption: &'a str,
    /// The points along the axis, in order.
    labels: &'a [String],
    /// A line each, with a value per label.
    series: Vec<Series<'a>>,
    /// How many points apart the axis labels them.
    every: usize,
) -> Result<impl View> {
    let max = series
        .iter()
        .flat_map(|series| series.values.iter().copied())
        .fold(0.0, f64::max);
    let scale = Scale::of(max);
    let base = HEIGHT - BOTTOM;
    let step = (WIDTH - LEFT - RIGHT) / float(labels.len().saturating_sub(1).max(1));
    let x = move |i: usize| LEFT + step * float(i);
    let lines: Vec<Line<'_>> = series
        .iter()
        .enumerate()
        .map(|(n, series)| {
            let mut path = String::new();
            for (i, value) in series.values.iter().enumerate() {
                let command = if i == 0 { 'M' } else { 'L' };
                write!(path, "{command}{:.1},{:.1}", x(i), scale.y(*value)).ok();
            }
            let last = series.values.len().saturating_sub(1);
            let value = series.values.last().copied().unwrap_or(0.0);
            Line {
                class: format!("s{}", n + 1),
                path,
                end: (x(last), scale.y(value)),
                tip: format!(
                    "{}, {}: {}",
                    series.name,
                    labels.get(last).map_or("", String::as_str),
                    compact(value)
                ),
                name: series.name,
            }
        })
        .collect();
    Ok(view! {
        <figure class="chart">
            <ul class="legend">
                for line in &lines {
                    <li><span class=(format!("key {}", line.class))></span>(line.name)</li>
                }
            </ul>
            <div class="chart-frame" tabindex="0">
                <svg viewBox=(format!("0 0 {WIDTH} {HEIGHT}")) role="img" aria-label=(caption)>
                    value_axis(scale: &scale)
                    for (i, label) in labels.iter().enumerate() {
                        if labelled(i, labels.len(), every) {
                            <text class="axis" x=(format!("{:.1}", x(i))) y=(format!("{}", HEIGHT - 6.0)) text-anchor=(if i == 0 { "start" } else if i + 1 == labels.len() { "end" } else { "middle" })>(label.as_str())</text>
                        }
                    }
                    <line class="baseline" x1=(format!("{LEFT}")) x2=(format!("{}", WIDTH - RIGHT)) y1=(format!("{base}")) y2=(format!("{base}"))></line>
                    for line in &lines {
                        <path class=(format!("line {}", line.class)) d=(line.path.as_str())></path>
                        <circle class=(format!("marker {}", line.class)) cx=(format!("{:.1}", line.end.0)) cy=(format!("{:.1}", line.end.1)) r="4"><title>(line.tip.as_str())</title></circle>
                    }
                </svg>
            </div>
            chart_table(caption: caption, labels: labels.iter().map(String::as_str).collect(), columns: series.iter().map(|series| (series.name, series.values.iter().map(|value| compact(*value)).collect())).collect())
        </figure>
    })
}

/// A horizontal bar per point, longest for the largest, with what it says at
/// its tip: for counts of a few named things.
#[component]
pub async fn bar_chart<'a>(
    /// What the chart shows, for screen readers and the table.
    caption: &'a str,
    /// A bar each, in order.
    points: &'a [Bar],
) -> Result<impl View> {
    const WIDTH: f64 = 440.0;
    const ROW: f64 = 30.0;
    const LABEL: f64 = 160.0;
    const THICK: f64 = 16.0;
    const VALUE: f64 = 56.0;
    let height = ROW * float(points.len().max(1));
    let max = points
        .iter()
        .map(|point| point.value)
        .fold(0.0, f64::max)
        .max(1.0);
    let bars: Vec<(f64, String, String, &Bar)> = points
        .iter()
        .enumerate()
        .map(|(i, point)| {
            let middle = ROW * float(i) + ROW / 2.0;
            let length = (point.value / max) * (WIDTH - LABEL - VALUE);
            let (top, end) = (middle - THICK / 2.0, LABEL + length);
            let radius = ROUND.min(length / 2.0).max(0.0);
            let path = format!(
                "M{LABEL},{top:.1}H{:.1}Q{end:.1},{top:.1} {end:.1},{:.1}V{:.1}Q{end:.1},{:.1} {:.1},{:.1}H{LABEL}Z",
                end - radius,
                top + radius,
                top + THICK - radius,
                top + THICK,
                end - radius,
                top + THICK,
            );
            (middle, path, format!("{:.1}", end + 6.0), point)
        })
        .collect();
    Ok(view! {
        <figure class="chart">
            <div class="chart-frame" tabindex="0">
                <svg class="bars" viewBox=(format!("0 0 {WIDTH} {height}")) role="img" aria-label=(caption)>
                    for (middle, path, tip_x, point) in &bars {
                        let y = format!("{middle:.1}");
                        <g>
                            <title>(format!("{}: {}", point.label, point.shown))</title>
                            <text class="label" x="0" y=(y.clone()) dominant-baseline="middle">(point.label.as_str())</text>
                            if point.value > 0.0 {
                                <path class="mark s1" d=(path.as_str())></path>
                            }
                            <text class="value" x=(tip_x.as_str()) y=(y) dominant-baseline="middle">(point.shown.as_str())</text>
                        </g>
                    }
                </svg>
            </div>
            chart_table(caption: caption, labels: points.iter().map(|point| point.label.as_str()).collect(), columns: vec![(caption, points.iter().map(|point| point.shown.clone()).collect())])
        </figure>
    })
}

/// What a chart shows, as a table a screen reader or a curious reader opens.
#[component]
async fn chart_table<'a>(
    /// What the chart shows.
    caption: &'a str,
    /// Each row's label.
    labels: Vec<&'a str>,
    /// Each column's name, and a cell per row.
    columns: Vec<(&'a str, Vec<String>)>,
) -> Result<impl View> {
    Ok(view! {
        <details class="chart-table">
            <summary>"Show as a table"<span class="visually-hidden">(format!(": {caption}"))</span></summary>
            <table>
                <caption class="visually-hidden">(caption)</caption>
                <thead>
                    <tr>
                        <td></td>
                        for (name, _) in &columns {
                            <th scope="col">(*name)</th>
                        }
                    </tr>
                </thead>
                <tbody>
                    for (i, label) in labels.iter().enumerate() {
                        <tr>
                            <th scope="row">(*label)</th>
                            for (_, cells) in &columns {
                                <td>(cells.get(i).map_or("", String::as_str))</td>
                            }
                        </tr>
                    }
                </tbody>
            </table>
        </details>
    })
}

#[cfg(test)]
mod tests;
