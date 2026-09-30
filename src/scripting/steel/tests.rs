use std::time::{Duration, Instant};

use super::*;
use crate::telemetry::RawTelemetry;

fn snapshot(aoa: f64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        revision: 42,
        received_at: Instant::now(),
        status: TelemetryStatus::Active,
        ias_kmh: Some(321.4),
        tas_kmh: Some(350.2),
        aoa_deg: Some(aoa),
        overload_g: Some(4.25),
        altitude_m: Some(1234.0),
        vertical_speed_ms: Some(-12.3),
        raw: RawTelemetry::default(),
    }
}

fn timed_snapshot(revision: u64, received_at: Instant, ias_kmh: f64) -> TelemetrySnapshot {
    let mut snapshot = snapshot(0.0);
    snapshot.revision = revision;
    snapshot.received_at = received_at;
    snapshot.ias_kmh = Some(ias_kmh);
    snapshot
}

#[test]
fn example_script_produces_the_mvp_nodes_and_warning() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(include_str!("../../../hud.example.scm"))
        .unwrap();
    let scene = engine.evaluate(&snapshot(19.0)).unwrap();
    assert_eq!(scene.revision, 42);
    assert_eq!(scene.nodes.len(), 7);
    assert_eq!(scene.nodes[0].id, "ias");
    assert_eq!(scene.nodes[0].slot, Slot::CrosshairRight);
    assert_eq!(scene.nodes[0].text, "IAS 321 km/h");
    assert_eq!(scene.nodes[6].id, "high-aoa");
    assert_eq!(scene.nodes[6].style.weight, FontWeight::Bold);
    assert_eq!(scene.nodes[6].style.blink_hz, Some(3.0));
}

#[test]
fn example_script_formats_integer_fallbacks_before_telemetry_connects() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(include_str!("../../../hud.example.scm"))
        .unwrap();
    let disconnected = TelemetrySnapshot::with_status(
        1,
        Instant::now(),
        TelemetryStatus::Disconnected,
        RawTelemetry::default(),
    );

    let scene = engine.evaluate(&disconnected).unwrap();
    assert!(scene.nodes.is_empty());
}

#[test]
fn hangar_snapshot_always_produces_an_empty_scene() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(include_str!("../../../hud.example.scm"))
        .unwrap();
    let mut hangar = snapshot(50.0);
    hangar.status = TelemetryStatus::Hangar;

    let scene = engine.evaluate(&hangar).unwrap();
    assert!(scene.nodes.is_empty());
}

#[test]
fn initial_evaluation_failure_retries_staged_script() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load("(define (build-hud t) (list (text 'value (slot (if (number? (telemetry t 'ias-kmh)) 'crosshair-top 'invalid)) (value \"ok\"))))")
        .unwrap();
    let disconnected = TelemetrySnapshot::with_status(
        1,
        Instant::now(),
        TelemetryStatus::Disconnected,
        RawTelemetry::default(),
    );

    assert!(engine.evaluate(&disconnected).is_err());
    assert_eq!(
        engine.evaluate(&snapshot(0.0)).unwrap().nodes[0].id,
        "value"
    );
}

#[test]
fn invisible_nodes_are_omitted() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(include_str!("../../../hud.example.scm"))
        .unwrap();
    let scene = engine.evaluate(&snapshot(5.0)).unwrap();
    assert_eq!(scene.nodes.len(), 6);
    assert!(scene.nodes.iter().all(|node| node.id != "high-aoa"));
}

#[test]
fn invalid_reload_keeps_the_previous_program() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load("(define (build-hud t) (list (text 'ok (slot 'crosshair-top) (value \"ok\"))))")
        .unwrap();
    assert!(engine.load("(define (build-hud").is_err());
    let scene = engine.evaluate(&snapshot(0.0)).unwrap();
    assert_eq!(scene.nodes[0].id, "ok");
}

#[test]
fn validates_duplicate_ids() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load("(define (build-hud t) (list (text 'same (slot 'crosshair-top) (value \"one\")) (text 'same (slot 'crosshair-bottom) (value \"two\"))))")
        .unwrap();
    assert!(engine.evaluate(&snapshot(0.0)).is_err());
}
#[test]
fn evaluation_failure_keeps_the_previous_program() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load("(define (build-hud t) (list (text 'old (slot 'crosshair-top) (value \"old\"))))")
        .unwrap();
    assert_eq!(engine.evaluate(&snapshot(0.0)).unwrap().nodes[0].id, "old");
    engine
        .load("(define (build-hud t) (list (text 'bad (slot 'not-a-slot) (value \"bad\"))))")
        .unwrap();
    assert!(engine.evaluate(&snapshot(0.0)).is_err());
    assert_eq!(engine.evaluate(&snapshot(0.0)).unwrap().nodes[0].id, "old");
}

#[test]
fn scripts_can_compute_generic_deltas_and_rates_from_history() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(
            r#"
              (define (ias sample) (telemetry sample 'ias-kmh))
              (define (build-hud t)
                (let ((samples (history t 'ias (telemetry t 'ias-kmh) 2000)))
                  (list
                    (text 'delta (slot 'crosshair-top)
                      (value (format-value "" (series-delta samples sample-value) 0 "")))
                    (text 'rate (slot 'crosshair-bottom)
                      (value (format-value "" (series-rate samples sample-value) 0 ""))))))
            "#,
        )
        .unwrap();
    let start = Instant::now();

    assert!(engine.evaluate(&timed_snapshot(1, start, 320.0)).is_err());
    let scene = engine
        .evaluate(&timed_snapshot(2, start + Duration::from_secs(1), 280.0))
        .unwrap();

    assert_eq!(scene.nodes[0].text, "-40");
    assert_eq!(scene.nodes[1].text, "-40");
}

#[test]
fn history_supports_raw_telemetry_fields() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(
            r#"
              (define (build-hud t)
                (let ((delta
                        (series-delta
                          (history t 'fuel (telemetry t 'state "fuel") 5000)
                          sample-value)))
                  (list (text 'fuel (slot 'crosshair-top)
                    (value (format-value "" delta 0 ""))))))
            "#,
        )
        .unwrap();
    let start = Instant::now();
    let mut first = timed_snapshot(1, start, 300.0);
    first.raw.state.insert("fuel".into(), 100.into());
    let mut second = timed_snapshot(2, start + Duration::from_secs(1), 300.0);
    second.raw.state.insert("fuel".into(), 91.into());

    assert!(engine.evaluate(&first).is_err());
    let scene = engine.evaluate(&second).unwrap();
    assert_eq!(scene.nodes[0].text, "-9");
}

#[test]
fn example_speed_loss_warning_waits_for_enough_history() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(include_str!("../../../hud.example.scm"))
        .unwrap();
    let start = Instant::now();

    let early = engine
        .evaluate(&timed_snapshot(
            1,
            start + Duration::from_millis(100),
            300.0,
        ))
        .unwrap();
    assert!(early.nodes.iter().all(|node| node.id != "rapid-speed-loss"));

    engine
        .evaluate(&timed_snapshot(2, start + Duration::from_secs(1), 280.0))
        .unwrap();
    let warned = engine
        .evaluate(&timed_snapshot(3, start + Duration::from_secs(2), 240.0))
        .unwrap();

    let warning = warned
        .nodes
        .iter()
        .find(|node| node.id == "rapid-speed-loss")
        .unwrap();
    assert_eq!(warning.text, "БЫСТРАЯ ПОТЕРЯ СКОРОСТИ -60 km/h");
    assert_eq!(warning.style.blink_hz, Some(3.0));
}

#[test]
fn example_speed_loss_warning_ignores_missing_ias() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(include_str!("../../../hud.example.scm"))
        .unwrap();
    let start = Instant::now();

    engine.evaluate(&timed_snapshot(1, start, 300.0)).unwrap();
    engine
        .evaluate(&timed_snapshot(2, start + Duration::from_secs(1), 300.0))
        .unwrap();
    let mut missing = timed_snapshot(3, start + Duration::from_secs(2), 0.0);
    missing.ias_kmh = None;
    let scene = engine.evaluate(&missing).unwrap();

    assert!(scene.nodes.iter().all(|node| node.id != "rapid-speed-loss"));
}

#[test]
fn script_reload_preserves_telemetry_history() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(
            r#"
              (define (build-hud t)
                (let ((tracked (history t 'ias (telemetry t 'ias-kmh) 2000)))
                  (list)))
            "#,
        )
        .unwrap();
    let start = Instant::now();
    engine.evaluate(&timed_snapshot(1, start, 300.0)).unwrap();

    engine
        .load(
            r#"
              (define (build-hud t)
                (list (text 'delta (slot 'crosshair-top)
                  (value
                    (format-value ""
                      (series-delta
                        (history t 'ias (telemetry t 'ias-kmh) 2000)
                        sample-value)
                      0 "")))))
            "#,
        )
        .unwrap();
    let scene = engine
        .evaluate(&timed_snapshot(2, start + Duration::from_secs(1), 275.0))
        .unwrap();

    assert_eq!(scene.nodes[0].text, "-25");
}

#[test]
fn stateful_helpers_are_available_to_scripts() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(
            r#"
              (define plain (style (shadow #f)))
              (define (build-hud t)
                (let ((raw (telemetry t 'ias-kmh))
                      (dangerous (> (telemetry t 'ias-kmh) 150)))
                  (let ((smooth (ema-filter 'smooth-ias raw 0.5))
                        (edge (trigger-edge 'danger-edge dangerous 'rising))
                        (held (hold 'danger-hold dangerous 1000))
                        (stable (debounce 'danger-stable dangerous 500)))
                    (list
                      (text 'smooth (slot 'crosshair-top)
                        (value (format-value "" smooth 0 "")))
                      (text 'edge (slot 'crosshair-top) (value "edge")
                        (visible edge))
                      (text 'held (slot 'crosshair-top) (value "held")
                        (visible held))
                      (text 'stable (slot 'crosshair-top) (value "stable")
                        (visible stable))
                      (text 'math (slot 'crosshair-top)
                        (value (format-value ""
                          (+ (clamp 12 0 10) (lerp 0 10 0.5)) 0 "")))))))
            "#,
        )
        .unwrap();
    let start = Instant::now();

    let initial = engine.evaluate(&timed_snapshot(1, start, 100.0)).unwrap();
    assert_eq!(initial.nodes[0].text, "100");
    assert_eq!(initial.nodes[1].id, "math");
    assert_eq!(initial.nodes[1].text, "15");

    let rising = engine
        .evaluate(&timed_snapshot(
            2,
            start + Duration::from_millis(100),
            200.0,
        ))
        .unwrap();
    assert_eq!(rising.nodes[0].text, "150");
    assert!(rising.nodes.iter().any(|node| node.id == "edge"));
    assert!(rising.nodes.iter().any(|node| node.id == "held"));
    assert!(rising.nodes.iter().all(|node| node.id != "stable"));

    let debounced = engine
        .evaluate(&timed_snapshot(
            3,
            start + Duration::from_millis(700),
            200.0,
        ))
        .unwrap();
    assert!(debounced.nodes.iter().all(|node| node.id != "edge"));
    assert!(debounced.nodes.iter().any(|node| node.id == "stable"));
}

#[test]
fn scripts_can_interpolate_colors_and_complete_styles() {
    let mut engine = SteelHudScriptEngine::new();
    engine
        .load(
            r##"
              (define calm
                (style (foreground "#000000") (font-size 10)
                  (font-weight 'normal) (shadow #f) (blink-hz #f)))
              (define danger
                (style (foreground "#ffffff") (font-size 30)
                  (font-weight 'bold) (shadow "#ff0000") (blink-hz 4)))
              (define (build-hud t)
                (list
                  (text 'blended (slot 'crosshair-top) (value "blend")
                    (text-style (style-blend calm danger 0.5)))
                  (text 'color (slot 'crosshair-bottom)
                    (value (color-lerp "#00ff00" "#ff0000" 0.5)))))
            "##,
        )
        .unwrap();

    let scene = engine.evaluate(&snapshot(0.0)).unwrap();
    let blended = &scene.nodes[0].style;
    assert!((blended.foreground.red - 128.0 / 255.0).abs() < 0.001);
    assert!((blended.foreground.green - 128.0 / 255.0).abs() < 0.001);
    assert_eq!(blended.font_size, 20.0);
    assert_eq!(blended.weight, FontWeight::Bold);
    assert_eq!(blended.blink_hz, Some(4.0));
    assert!((blended.shadow.unwrap().alpha - 128.0 / 255.0).abs() < 0.001);
    assert_eq!(scene.nodes[1].text, "#808000");
}
