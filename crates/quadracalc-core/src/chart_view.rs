//! Insert › Charts/PivotChart: turns the active sheet's own GridCraft
//! `Chart`s into quadraui [`Chart`] primitives so [`crate::app::View::screen`]
//! can push them as `Surface::Chart`, the same way it already pushes the
//! cell grid as `Surface::Widget`/`TabBar` (see `app.rs`'s module doc).
//! Before this module existed, `insert.chart`/`insert.pivotChart` ran
//! fine against the engine — the command surface never needed a
//! parameter this app couldn't supply — but nothing ever painted the
//! result: #20 is this gap.
//!
//! The chart list itself is a plain read of `Session::doc` (same
//! "structural view data" exception `crate::sheets` documents), but each
//! series' actual numbers are a formula range (`Chart::series[].values`,
//! e.g. `"Sheet1!$A$1:$A$5"`) that only the calc engine can resolve —
//! that goes through `sheet.read`, GridCraft's own command, rather than
//! this app reaching into `gridcraft_chart::resolve` (an engine-internal
//! crate quadracalc doesn't depend on and CLAUDE.md's "only through its
//! command surface" rule keeps it that way).
//!
//! quadraui's own [`ChartKind`] has four shapes (sparkline, line, bar,
//! grouped bar); GridCraft's model has many more (pie, scatter, radar,
//! …). [`quadraui_kind`] degrades anything quadraui can't draw natively
//! to the closest of those four rather than disabling the whole Insert
//! item — a rough chart this app can paint beats none at all.

use gridcraft_engine::model::ChartKind as EngineChartKind;
use gridcraft_engine::{Session, model};
use quadraui::{Chart, ChartKind, Series, WidgetId};
use serde_json::{Value, json};

/// Every chart on the active sheet, in sheet order — empty with no
/// document open, or no charts on it.
pub fn charts(session: &mut Session) -> Vec<Chart> {
    let engine_charts: Vec<model::Chart> = session
        .doc()
        .ok()
        .and_then(|d| d.wb.active())
        .map(|sh| sh.charts.clone())
        .unwrap_or_default();
    engine_charts.iter().map(|c| build(session, c)).collect()
}

fn build(session: &mut Session, chart: &model::Chart) -> Chart {
    let fill = matches!(
        chart.kind,
        EngineChartKind::Area | EngineChartKind::AreaStacked | EngineChartKind::AreaStacked100
    );
    let series = chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| Series {
            label: s
                .name
                .as_deref()
                .and_then(|r| read_one(session, r))
                .unwrap_or_else(|| format!("Series {}", i + 1)),
            data: read_numbers(session, &s.values),
            color: None,
            fill,
        })
        .collect();
    Chart {
        id: WidgetId::new(format!("quadracalc-chart-{}", chart.id)),
        kind: quadraui_kind(chart.kind),
        series,
        // `Chart` (quadraui) has no title field of its own, only axis
        // labels — the chart's own title is the closer match for what a
        // viewer expects to read first, so it wins over `x_title` rather
        // than the two being concatenated or either being dropped.
        x_label: chart.title.clone().or_else(|| chart.x_title.clone()),
        y_label: chart.y_title.clone(),
        y_range: None,
        x_range: None,
        show_legend: !matches!(chart.legend, model::LegendPos::None),
        y_ticks: None,
        x_ticks: None,
        show_grid: chart.gridlines,
    }
}

/// Reads `range` (a formula-text cell/range reference, e.g.
/// `"Sheet1!$A$1:$A$5"`) through `sheet.read` and flattens every numeric
/// cell into one series, row-major — good enough for the single-row or
/// single-column ranges `insert.chart` itself builds from a selection; a
/// text or blank cell in the range reads as `0.0` rather than being
/// skipped, so the series stays the same length as the range (a chart
/// axis with a gap is still meaningful; one that silently shifted
/// wouldn't be).
fn read_numbers(session: &mut Session, range: &str) -> Vec<f64> {
    let Ok(result) = session.execute("sheet.read", json!({ "range": range })) else {
        return Vec::new();
    };
    result
        .get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .collect()
}

/// The first cell `range` resolves to, as display text — used for a
/// series' name reference (`Series::name`, e.g. `"Sheet1!$B$1"`) rather
/// than [`read_numbers`], since a name is text to show, not a value to
/// plot. `None` on any read failure or an empty range, so the caller
/// falls back to a generic "Series N" label instead of an empty one.
fn read_one(session: &mut Session, range: &str) -> Option<String> {
    let result = session
        .execute("sheet.read", json!({ "range": range }))
        .ok()?;
    let first = result
        .get("values")?
        .as_array()?
        .first()?
        .as_array()?
        .first()?;
    match first {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The closest quadraui [`ChartKind`] a GridCraft [`EngineChartKind`]
/// degrades to — see this module's own doc for why a rough match beats
/// disabling the whole Insert item. Side-by-side column/bar layouts
/// become [`ChartKind::BarGrouped`]; every other bar-shaped kind
/// (including a 100%-stacked one, which this degrade doesn't attempt to
/// renormalize) becomes the stacked [`ChartKind::Bar`]; everything else
/// — line, area, and every kind quadraui has no bar/line analogue for at
/// all (pie, scatter, radar, box & whisker, …) — becomes
/// [`ChartKind::Line`], the shape every one of those at least has an
/// x/y value series in common with.
fn quadraui_kind(kind: EngineChartKind) -> ChartKind {
    match kind {
        EngineChartKind::ColumnClustered | EngineChartKind::BarClustered => ChartKind::BarGrouped,
        EngineChartKind::ColumnStacked
        | EngineChartKind::ColumnStacked100
        | EngineChartKind::BarStacked
        | EngineChartKind::BarStacked100
        | EngineChartKind::Histogram
        | EngineChartKind::Waterfall
        | EngineChartKind::Funnel => ChartKind::Bar,
        _ => ChartKind::Line,
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;
    use serde_json::json;

    use super::*;

    fn session_with_data() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s.execute(
            "range.setValues",
            json!({"range": "A1:B3", "values": [["Q1", 10], ["Q2", 20], ["Q3", 30]]}),
        )
        .unwrap();
        s
    }

    #[test]
    fn no_document_yields_no_charts() {
        let mut s = Session::new();
        assert_eq!(charts(&mut s), Vec::new());
    }

    #[test]
    fn an_inserted_chart_resolves_its_series_values() {
        let mut s = session_with_data();
        s.execute("insert.chart", json!({"range": "A1:B3", "type": "line"}))
            .unwrap();
        let built = charts(&mut s);
        assert_eq!(built.len(), 1);
        assert_eq!(built[0].kind, ChartKind::Line);
        assert_eq!(built[0].series.len(), 1);
        assert_eq!(built[0].series[0].data, vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn a_column_chart_degrades_to_a_grouped_bar() {
        let mut s = session_with_data();
        s.execute("insert.chart", json!({"range": "A1:B3", "type": "column"}))
            .unwrap();
        assert_eq!(charts(&mut s)[0].kind, ChartKind::BarGrouped);
    }

    #[test]
    fn a_pie_chart_degrades_to_a_line_rather_than_disappearing() {
        let mut s = session_with_data();
        s.execute("insert.chart", json!({"range": "A1:B3", "type": "pie"}))
            .unwrap();
        assert_eq!(charts(&mut s)[0].kind, ChartKind::Line);
    }
}
