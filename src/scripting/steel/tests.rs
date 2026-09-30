use std::time::Instant;

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
