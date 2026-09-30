use std::{collections::HashMap, str::FromStr, time::Duration};

use serde_json::Value;
use steel::{
    SteelVal,
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};

use crate::{
    scene::{FontWeight, Rgba, Slot, TextNode, TextScene, TextStyle},
    telemetry::{TelemetrySnapshot, TelemetryStatus},
};

use super::{
    HudScriptEngine, ScriptError,
    history::{MAX_HISTORY, SharedMetricHistory},
    stateful::{EdgeDirection, MAX_STATE_DURATION, SharedStatefulValues},
};

const HOST_API: &str = r#"
  (define (slot name) (list 'slot name))
  (define (value content) (list 'value content))
  (define (visible flag) (list 'visible flag))
  (define (text-style value) (list 'text-style value))

  (define (foreground color) (list 'foreground color))
  (define (font-size size) (list 'font-size size))
  (define (font-weight weight) (list 'font-weight weight))
  (define (shadow color) (list 'shadow color))
  (define (blink-hz frequency) (list 'blink-hz frequency))

  (define (style . properties) (cons 'style properties))
  (define (text id . properties) (cons 'text (cons id properties)))

  (define (hud-lookup entries key)
    (if (null? entries)
        #f
        (if (equal? (car (car entries)) key)
            (car (cdr (car entries)))
            (hud-lookup (cdr entries) key))))

  ;; With two arguments this reads a stable normalized field. With three it
  ;; reads `(telemetry t 'state "raw field")` or the indicators equivalent.
  (define (telemetry snapshot field . raw-key)
    (if (null? raw-key)
        (hud-lookup snapshot field)
        (hud-lookup (hud-lookup snapshot field) (car raw-key))))

  (define (sample-value sample) (telemetry sample 'value))

  ;; History is ordered from oldest to newest. Accessors may read any
  ;; normalized or raw telemetry field from each sample.
  (define (hud-series-first samples accessor)
    (if (null? samples)
        #f
        (let ((value (accessor (car samples))))
          (if (number? value)
              (list (telemetry (car samples) 'age-ms) value)
              (hud-series-first (cdr samples) accessor)))))

  (define (hud-series-last samples accessor found)
    (if (null? samples)
        found
        (let ((value (accessor (car samples))))
          (hud-series-last
            (cdr samples)
            accessor
            (if (number? value)
                (list (telemetry (car samples) 'age-ms) value)
                found)))))

  (define (series-delta samples accessor)
    (let ((first (hud-series-first samples accessor))
          (last (hud-series-last samples accessor #f)))
      (if (and first last (> (car first) (car last)))
          (- (car (cdr last)) (car (cdr first)))
          #f)))

  (define (series-rate samples accessor)
    (let ((first (hud-series-first samples accessor))
          (last (hud-series-last samples accessor #f)))
      (if (and first last)
          (let ((elapsed-ms (- (car first) (car last))))
            (if (> elapsed-ms 0)
                (/ (* 1000 (- (car (cdr last)) (car (cdr first)))) elapsed-ms)
                #f))
          #f)))

  (define (series-span-ms samples accessor)
    (let ((first (hud-series-first samples accessor))
          (last (hud-series-last samples accessor #f)))
      (if (and first last)
          (- (car first) (car last))
          0)))

  (define (hud-series-stats samples accessor count sum minimum maximum)
    (if (null? samples)
        (list count sum minimum maximum)
        (let ((value (accessor (car samples))))
          (if (number? value)
              (hud-series-stats
                (cdr samples) accessor (+ count 1) (+ sum value)
                (if (or (not minimum) (< value minimum)) value minimum)
                (if (or (not maximum) (> value maximum)) value maximum))
              (hud-series-stats
                (cdr samples) accessor count sum minimum maximum)))))

  (define (series-average samples accessor)
    (let ((stats (hud-series-stats samples accessor 0 0 #f #f)))
      (if (> (car stats) 0)
          (/ (car (cdr stats)) (car stats))
          #f)))

  (define (series-min samples accessor)
    (car (cdr (cdr (hud-series-stats samples accessor 0 0 #f #f)))))

  (define (series-max samples accessor)
    (car (cdr (cdr (cdr (hud-series-stats samples accessor 0 0 #f #f))))))
"#;

/// Sandboxed Steel adapter. A successful first evaluation activates the staged engine, so
/// definitions removed from a reloaded file cannot leak from the old program.
pub struct SteelHudScriptEngine {
    active: Option<Engine>,
    staged: Option<Engine>,
    history: SharedMetricHistory,
    stateful: SharedStatefulValues,
}

impl Default for SteelHudScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SteelHudScriptEngine {
    pub fn new() -> Self {
        Self {
            active: None,
            staged: None,
            history: SharedMetricHistory::default(),
            stateful: SharedStatefulValues::default(),
        }
    }

    fn candidate(
        history: SharedMetricHistory,
        stateful: SharedStatefulValues,
    ) -> Result<Engine, ScriptError> {
        let mut engine = Engine::new_sandboxed();
        engine.register_fn(
            "format-value",
            |prefix: String, value: SteelVal, precision: SteelVal, suffix: String| {
                format_value(&prefix, &value, &precision, &suffix)
            },
        );
        engine.register_fn(
            "history",
            move |_telemetry: SteelVal, name: SteelVal, value: SteelVal, window_ms: SteelVal| {
                history_value(&history, &name, &value, &window_ms)
            },
        );
        let edge_state = stateful.clone();
        engine.register_fn(
            "trigger-edge",
            move |key: SteelVal, value: SteelVal, direction: SteelVal| {
                trigger_edge_value(&edge_state, &key, &value, &direction)
            },
        );
        let hold_state = stateful.clone();
        engine.register_fn(
            "hold",
            move |key: SteelVal, value: SteelVal, duration_ms: SteelVal| {
                hold_value(&hold_state, &key, &value, &duration_ms)
            },
        );
        let debounce_state = stateful.clone();
        engine.register_fn(
            "debounce",
            move |key: SteelVal, value: SteelVal, duration_ms: SteelVal| {
                debounce_value(&debounce_state, &key, &value, &duration_ms)
            },
        );
        engine.register_fn(
            "ema-filter",
            move |key: SteelVal, value: SteelVal, alpha: SteelVal| {
                ema_value(&stateful, &key, &value, &alpha)
            },
        );
        engine.register_fn(
            "clamp",
            |value: SteelVal, minimum: SteelVal, maximum: SteelVal| {
                clamp_value(&value, &minimum, &maximum)
            },
        );
        engine.register_fn(
            "lerp",
            |start: SteelVal, end: SteelVal, amount: SteelVal| lerp_value(&start, &end, &amount),
        );
        engine.register_fn(
            "color-lerp",
            |start: String, end: String, amount: SteelVal| color_lerp_value(&start, &end, &amount),
        );
        engine.register_fn(
            "style-blend",
            |start: SteelVal, end: SteelVal, amount: SteelVal| {
                style_blend_value(&start, &end, &amount)
            },
        );
        engine
            .run(HOST_API)
            .map_err(|error| steel_error(&engine, error, true))?;
        Ok(engine)
    }
}

impl HudScriptEngine for SteelHudScriptEngine {
    fn load(&mut self, source: &str) -> Result<(), ScriptError> {
        let mut candidate = Self::candidate(self.history.clone(), self.stateful.clone())?;
        candidate
            .run(source.to_owned())
            .map_err(|error| steel_error(&candidate, error, true))?;

        let entry = candidate
            .extract_value("build-hud")
            .map_err(|error| steel_error(&candidate, error, true))?;
        if !matches!(
            entry,
            SteelVal::Closure(_) | SteelVal::BoxedFunction(_) | SteelVal::FuncV(_)
        ) {
            return Err(ScriptError::compile(
                "`build-hud` must be defined as a function taking one telemetry value",
            ));
        }

        // Activation is completed by the first successful evaluation. This
        // lets a reload with a runtime/type error preserve the prior program.
        self.staged = Some(candidate);
        Ok(())
    }

    fn evaluate(&mut self, telemetry: &TelemetrySnapshot) -> Result<TextScene, ScriptError> {
        self.history.begin_evaluation(telemetry);
        self.stateful.begin_evaluation(telemetry);
        if telemetry.status == TelemetryStatus::Hangar {
            return Ok(TextScene {
                revision: telemetry.revision,
                nodes: Vec::new(),
            });
        }
        let argument = telemetry_value(telemetry, Duration::ZERO);
        if let Some(mut candidate) = self.staged.take() {
            let result = evaluate_engine(&mut candidate, argument.clone(), telemetry.revision);
            if result.is_ok() {
                self.active = Some(candidate);
            } else if self.active.is_none() {
                // Incomplete startup telemetry may cause the first evaluation
                // to fail. Retry this staged program on the next snapshot.
                self.staged = Some(candidate);
            }
            return result;
        }

        let engine = self.active.as_mut().ok_or_else(|| {
            ScriptError::evaluate("no successfully evaluated HUD script has been loaded")
        })?;
        evaluate_engine(engine, argument, telemetry.revision)
    }
}

fn evaluate_engine(
    engine: &mut Engine,
    argument: SteelVal,
    revision: u64,
) -> Result<TextScene, ScriptError> {
    let result = engine
        .call_function_by_name_with_args("build-hud", vec![argument])
        .map_err(|error| steel_error(engine, error, false))?;
    decode_scene(result, revision)
}

fn steel_error(engine: &Engine, error: steel::SteelErr, compile: bool) -> ScriptError {
    let message = engine
        .raise_error_to_string(error.clone())
        .unwrap_or_else(|| error.to_string());
    if compile {
        ScriptError::compile(message)
    } else {
        ScriptError::evaluate(message)
    }
}

fn format_value(
    prefix: &str,
    value: &SteelVal,
    precision: &SteelVal,
    suffix: &str,
) -> Result<String, String> {
    let value = steel_number(value, "value")?;
    let precision = steel_integer(precision, "precision")?.clamp(0, 6) as usize;
    Ok(format!("{prefix}{value:.precision$}{suffix}"))
}

fn steel_number(value: &SteelVal, name: &str) -> Result<f64, String> {
    match value {
        SteelVal::NumV(value) if value.is_finite() => Ok(*value),
        SteelVal::IntV(value) => Ok(*value as f64),
        _ => Err(format!(
            "format-value: {name} must be a finite number, got {value}"
        )),
    }
}

fn steel_integer(value: &SteelVal, name: &str) -> Result<isize, String> {
    match value {
        SteelVal::IntV(value) => Ok(*value),
        _ => Err(format!(
            "format-value: {name} must be an integer, got {value}"
        )),
    }
}

fn trigger_edge_value(
    stateful: &SharedStatefulValues,
    key: &SteelVal,
    value: &SteelVal,
    direction: &SteelVal,
) -> Result<bool, String> {
    let key = state_key(key, "trigger-edge")?;
    let value = state_bool(value, "trigger-edge", "value")?;
    let direction = match state_key(direction, "trigger-edge")?.as_str() {
        "rising" => EdgeDirection::Rising,
        "falling" => EdgeDirection::Falling,
        _ => return Err("trigger-edge: direction must be 'rising or 'falling".into()),
    };
    Ok(stateful.edge(&key, value, direction))
}

fn hold_value(
    stateful: &SharedStatefulValues,
    key: &SteelVal,
    value: &SteelVal,
    duration_ms: &SteelVal,
) -> Result<bool, String> {
    let key = state_key(key, "hold")?;
    let value = state_bool(value, "hold", "value")?;
    let duration = state_duration(duration_ms, "hold")?;
    Ok(stateful.hold(&key, value, duration))
}

fn debounce_value(
    stateful: &SharedStatefulValues,
    key: &SteelVal,
    value: &SteelVal,
    duration_ms: &SteelVal,
) -> Result<bool, String> {
    let key = state_key(key, "debounce")?;
    let value = state_bool(value, "debounce", "value")?;
    let duration = state_duration(duration_ms, "debounce")?;
    Ok(stateful.debounce(&key, value, duration))
}

fn ema_value(
    stateful: &SharedStatefulValues,
    key: &SteelVal,
    value: &SteelVal,
    alpha: &SteelVal,
) -> Result<SteelVal, String> {
    let key = state_key(key, "ema-filter")?;
    if matches!(value, SteelVal::BoolV(false)) {
        return Ok(SteelVal::BoolV(false));
    }
    let value = state_number(value, "ema-filter", "value")?;
    let alpha = state_number(alpha, "ema-filter", "alpha")?;
    if !(0.0..=1.0).contains(&alpha) {
        return Err("ema-filter: alpha must be between 0 and 1".into());
    }
    Ok(stateful
        .ema(&key, value, alpha)
        .map_or(SteelVal::BoolV(false), SteelVal::NumV))
}

fn clamp_value(value: &SteelVal, minimum: &SteelVal, maximum: &SteelVal) -> Result<f64, String> {
    let value = state_number(value, "clamp", "value")?;
    let minimum = state_number(minimum, "clamp", "minimum")?;
    let maximum = state_number(maximum, "clamp", "maximum")?;
    if minimum > maximum {
        return Err("clamp: minimum must not exceed maximum".into());
    }
    Ok(value.clamp(minimum, maximum))
}

fn lerp_value(start: &SteelVal, end: &SteelVal, amount: &SteelVal) -> Result<f64, String> {
    let start = state_number(start, "lerp", "start")?;
    let end = state_number(end, "lerp", "end")?;
    let amount = state_number(amount, "lerp", "amount")?;
    Ok(start + (end - start) * amount)
}

fn color_lerp_value(start: &str, end: &str, amount: &SteelVal) -> Result<String, String> {
    let start = Rgba::from_str(start).map_err(|error| format!("color-lerp: {error}"))?;
    let end = Rgba::from_str(end).map_err(|error| format!("color-lerp: {error}"))?;
    let amount = blend_amount(amount, "color-lerp")?;
    Ok(format_color(blend_color(start, end, amount)))
}

fn style_blend_value(
    start: &SteelVal,
    end: &SteelVal,
    amount: &SteelVal,
) -> Result<SteelVal, String> {
    let start = decode_style(start).map_err(|error| format!("style-blend: {error}"))?;
    let end = decode_style(end).map_err(|error| format!("style-blend: {error}"))?;
    let amount = blend_amount(amount, "style-blend")?;
    let style = TextStyle {
        foreground: blend_color(start.foreground, end.foreground, amount),
        font_size: lerp_f32(start.font_size, end.font_size, amount),
        weight: if amount < 0.5 {
            start.weight
        } else {
            end.weight
        },
        shadow: blend_shadow(start.shadow, end.shadow, amount),
        blink_hz: match (start.blink_hz, end.blink_hz) {
            (Some(start), Some(end)) => Some(lerp_f32(start, end, amount)),
            (start, end) => {
                if amount < 0.5 {
                    start
                } else {
                    end
                }
            }
        },
    };
    Ok(encode_style(&style))
}

fn blend_amount(value: &SteelVal, function: &str) -> Result<f64, String> {
    let amount = state_number(value, function, "factor")?;
    if !(0.0..=1.0).contains(&amount) {
        return Err(format!("{function}: factor must be between 0 and 1"));
    }
    Ok(amount)
}

fn blend_color(start: Rgba, end: Rgba, amount: f64) -> Rgba {
    Rgba {
        red: lerp_f32(start.red, end.red, amount),
        green: lerp_f32(start.green, end.green, amount),
        blue: lerp_f32(start.blue, end.blue, amount),
        alpha: lerp_f32(start.alpha, end.alpha, amount),
    }
}

fn blend_shadow(start: Option<Rgba>, end: Option<Rgba>, amount: f64) -> Option<Rgba> {
    match (start, end) {
        (Some(start), Some(end)) => Some(blend_color(start, end, amount)),
        (Some(mut start), None) if amount < 1.0 => {
            start.alpha *= 1.0 - amount as f32;
            Some(start)
        }
        (None, Some(mut end)) if amount > 0.0 => {
            end.alpha *= amount as f32;
            Some(end)
        }
        _ => None,
    }
}

fn lerp_f32(start: f32, end: f32, amount: f64) -> f32 {
    start + (end - start) * amount as f32
}

fn encode_style(style: &TextStyle) -> SteelVal {
    list([
        symbol("style"),
        list([
            symbol("foreground"),
            SteelVal::StringV(format_color(style.foreground).into()),
        ]),
        list([symbol("font-size"), SteelVal::NumV(style.font_size as f64)]),
        list([symbol("font-weight"), symbol(style.weight.as_str())]),
        list([
            symbol("shadow"),
            style.shadow.map_or(SteelVal::BoolV(false), |color| {
                SteelVal::StringV(format_color(color).into())
            }),
        ]),
        list([
            symbol("blink-hz"),
            style
                .blink_hz
                .map_or(SteelVal::BoolV(false), |value| SteelVal::NumV(value as f64)),
        ]),
    ])
}

fn format_color(color: Rgba) -> String {
    let component = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let red = component(color.red);
    let green = component(color.green);
    let blue = component(color.blue);
    let alpha = component(color.alpha);
    if alpha == u8::MAX {
        format!("#{red:02x}{green:02x}{blue:02x}")
    } else {
        format!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}")
    }
}

fn state_key(value: &SteelVal, function: &str) -> Result<String, String> {
    match value {
        SteelVal::StringV(value) | SteelVal::SymbolV(value) => Ok(value.to_string()),
        _ => Err(format!("{function}: key must be a symbol or string")),
    }
}

fn state_bool(value: &SteelVal, function: &str, name: &str) -> Result<bool, String> {
    match value {
        SteelVal::BoolV(value) => Ok(*value),
        _ => Err(format!("{function}: {name} must be a boolean")),
    }
}

fn state_number(value: &SteelVal, function: &str, name: &str) -> Result<f64, String> {
    match value {
        SteelVal::NumV(value) if value.is_finite() => Ok(*value),
        SteelVal::IntV(value) => Ok(*value as f64),
        _ => Err(format!("{function}: {name} must be a finite number")),
    }
}

fn state_duration(value: &SteelVal, function: &str) -> Result<Duration, String> {
    let milliseconds = state_number(value, function, "duration-ms")?;
    let maximum = MAX_STATE_DURATION.as_secs_f64() * 1000.0;
    if !(0.0..=maximum).contains(&milliseconds) {
        return Err(format!(
            "{function}: duration-ms must be between 0 and {maximum}"
        ));
    }
    Ok(Duration::from_secs_f64(milliseconds / 1000.0))
}

fn history_value(
    history: &SharedMetricHistory,
    name: &SteelVal,
    value: &SteelVal,
    window_ms: &SteelVal,
) -> Result<SteelVal, String> {
    let name = match name {
        SteelVal::StringV(value) | SteelVal::SymbolV(value) => value.to_string(),
        _ => return Err("history: name must be a symbol or string".into()),
    };
    let value = match value {
        SteelVal::BoolV(false) => None,
        SteelVal::NumV(value) if value.is_finite() => Some(*value),
        SteelVal::IntV(value) => Some(*value as f64),
        _ => return Err("history: value must be a finite number or #f".into()),
    };
    let window_ms = match window_ms {
        SteelVal::NumV(value) if value.is_finite() => *value,
        SteelVal::IntV(value) => *value as f64,
        _ => return Err("history: window-ms must be a finite number".into()),
    };
    let maximum_ms = MAX_HISTORY.as_secs_f64() * 1000.0;
    if !(0.0..=maximum_ms).contains(&window_ms) {
        return Err(format!(
            "history: window-ms must be between 0 and {maximum_ms}"
        ));
    }

    Ok(list(
        history
            .series(&name, value, Duration::from_secs_f64(window_ms / 1000.0))
            .iter()
            .map(|(age, value)| metric_sample_value(*age, *value)),
    ))
}

fn metric_sample_value(age: Duration, value: f64) -> SteelVal {
    list([
        entry("age-ms", SteelVal::NumV(age.as_secs_f64() * 1000.0)),
        entry("value", SteelVal::NumV(value)),
    ])
}

fn telemetry_value(snapshot: &TelemetrySnapshot, age: Duration) -> SteelVal {
    let status = match snapshot.status {
        TelemetryStatus::Disconnected => "disconnected",
        TelemetryStatus::Invalid => "invalid",
        TelemetryStatus::Hangar => "hangar",
        TelemetryStatus::Active => "active",
        TelemetryStatus::Stale => "stale",
    };
    list([
        entry("revision", SteelVal::IntV(snapshot.revision as isize)),
        entry("age-ms", SteelVal::NumV(age.as_secs_f64() * 1000.0)),
        entry("status", symbol(status)),
        entry("ias-kmh", optional_number(snapshot.ias_kmh)),
        entry("tas-kmh", optional_number(snapshot.tas_kmh)),
        entry("aoa-deg", optional_number(snapshot.aoa_deg)),
        entry("overload-g", optional_number(snapshot.overload_g)),
        entry("altitude-m", optional_number(snapshot.altitude_m)),
        entry(
            "vertical-speed-ms",
            optional_number(snapshot.vertical_speed_ms),
        ),
        entry("state", json_object(&snapshot.raw.state)),
        entry("indicators", json_object(&snapshot.raw.indicators)),
    ])
}

fn optional_number(value: Option<f64>) -> SteelVal {
    value
        .filter(|number| number.is_finite())
        .map_or(SteelVal::BoolV(false), SteelVal::NumV)
}

fn json_object(object: &serde_json::Map<String, Value>) -> SteelVal {
    list(
        object
            .iter()
            .map(|(key, value)| list([SteelVal::StringV(key.clone().into()), json_value(value)])),
    )
}

fn json_value(value: &Value) -> SteelVal {
    match value {
        Value::Null => SteelVal::BoolV(false),
        Value::Bool(value) => SteelVal::BoolV(*value),
        Value::Number(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map_or(SteelVal::BoolV(false), SteelVal::NumV),
        Value::String(value) => SteelVal::StringV(value.clone().into()),
        Value::Array(values) => list(values.iter().map(json_value)),
        Value::Object(object) => json_object(object),
    }
}

fn entry(name: &str, value: SteelVal) -> SteelVal {
    list([symbol(name), value])
}

fn symbol(value: &str) -> SteelVal {
    SteelVal::SymbolV(value.into())
}

fn list(values: impl IntoIterator<Item = SteelVal>) -> SteelVal {
    SteelVal::ListV(values.into_iter().collect())
}

fn decode_scene(value: SteelVal, revision: u64) -> Result<TextScene, ScriptError> {
    let nodes = list_items(&value, "build-hud result")?
        .iter()
        .filter_map(|node| match decode_node(node) {
            Ok(node) => node.map(Ok),
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scene = TextScene { revision, nodes };
    scene
        .validate()
        .map_err(|error| ScriptError::validation(error.to_string()))?;
    Ok(scene)
}

fn decode_node(value: &SteelVal) -> Result<Option<TextNode>, ScriptError> {
    let values = list_items(value, "text node")?;
    if values.len() < 2 || atom(values[0])? != "text" {
        return Err(ScriptError::validation(
            "each build-hud result item must be produced by `text`",
        ));
    }

    let id = atom(values[1])?;
    let mut properties = HashMap::new();
    for &property in &values[2..] {
        let pair = list_items(property, "text property")?;
        if pair.len() != 2 {
            return Err(ScriptError::validation(
                "a text property must contain its name and one value",
            ));
        }
        let name = atom(pair[0])?;
        if properties.insert(name.clone(), pair[1]).is_some() {
            return Err(ScriptError::validation(format!(
                "text node {id:?} repeats property {name:?}"
            )));
        }
    }

    if let Some(visible) = properties.remove("visible")
        && !boolean(visible)?
    {
        return Ok(None);
    }
    let slot = Slot::from_str(&atom(required(&mut properties, "slot", &id)?)?)
        .map_err(|error| ScriptError::validation(error.to_string()))?;
    let text = string(required(&mut properties, "value", &id)?)?;
    let style = properties
        .remove("text-style")
        .map_or_else(|| Ok(TextStyle::default()), decode_style)?;
    if let Some(name) = properties.keys().next() {
        return Err(ScriptError::validation(format!(
            "unknown property {name:?} on text node {id:?}"
        )));
    }

    Ok(Some(TextNode {
        id,
        slot,
        text,
        style,
    }))
}

fn required<'a>(
    properties: &mut HashMap<String, &'a SteelVal>,
    name: &str,
    id: &str,
) -> Result<&'a SteelVal, ScriptError> {
    properties
        .remove(name)
        .ok_or_else(|| ScriptError::validation(format!("text node {id:?} is missing `{name}`")))
}

fn decode_style(value: &SteelVal) -> Result<TextStyle, ScriptError> {
    let values = list_items(value, "style")?;
    if values
        .first()
        .map(|value| atom(value))
        .transpose()?
        .as_deref()
        != Some("style")
    {
        return Err(ScriptError::validation(
            "`text-style` expects a value produced by `style`",
        ));
    }

    let mut style = TextStyle::default();
    let mut seen = std::collections::HashSet::new();
    for &property in &values[1..] {
        let pair = list_items(property, "style property")?;
        if pair.len() != 2 {
            return Err(ScriptError::validation(
                "a style property must contain its name and one value",
            ));
        }
        let name = atom(pair[0])?;
        if !seen.insert(name.clone()) {
            return Err(ScriptError::validation(format!(
                "style repeats property {name:?}"
            )));
        }
        match name.as_str() {
            "foreground" => style.foreground = color(pair[1])?,
            "font-size" => style.font_size = number(pair[1], "font-size")? as f32,
            "font-weight" => {
                style.weight = FontWeight::from_str(&atom(pair[1])?)
                    .map_err(|error| ScriptError::validation(error.to_string()))?;
            }
            "shadow" => {
                style.shadow = if matches!(pair[1], SteelVal::BoolV(false)) {
                    None
                } else {
                    Some(color(pair[1])?)
                };
            }
            "blink-hz" => {
                style.blink_hz = if matches!(pair[1], SteelVal::BoolV(false)) {
                    None
                } else {
                    Some(number(pair[1], "blink-hz")? as f32)
                };
            }
            _ => {
                return Err(ScriptError::validation(format!(
                    "unknown style property {name:?}"
                )));
            }
        }
    }
    Ok(style)
}

fn color(value: &SteelVal) -> Result<Rgba, ScriptError> {
    let value = string(value)?;
    Rgba::from_str(&value).map_err(|error| ScriptError::validation(error.to_string()))
}

fn list_items<'a>(value: &'a SteelVal, context: &str) -> Result<Vec<&'a SteelVal>, ScriptError> {
    match value {
        SteelVal::ListV(values) => Ok(values.iter().collect()),
        _ => Err(ScriptError::validation(format!(
            "{context} must be a list, got {value}"
        ))),
    }
}

fn atom(value: &SteelVal) -> Result<String, ScriptError> {
    match value {
        SteelVal::StringV(value) | SteelVal::SymbolV(value) => Ok(value.to_string()),
        _ => Err(ScriptError::validation(format!(
            "expected a string or symbol, got {value}"
        ))),
    }
}

fn string(value: &SteelVal) -> Result<String, ScriptError> {
    match value {
        SteelVal::StringV(value) => Ok(value.to_string()),
        _ => Err(ScriptError::validation(format!(
            "expected a string, got {value}"
        ))),
    }
}

fn boolean(value: &SteelVal) -> Result<bool, ScriptError> {
    match value {
        SteelVal::BoolV(value) => Ok(*value),
        _ => Err(ScriptError::validation(format!(
            "expected a boolean, got {value}"
        ))),
    }
}

fn number(value: &SteelVal, name: &str) -> Result<f64, ScriptError> {
    let value = match value {
        SteelVal::NumV(value) => *value,
        SteelVal::IntV(value) => *value as f64,
        _ => {
            return Err(ScriptError::validation(format!(
                "`{name}` expects a number, got {value}"
            )));
        }
    };
    if !value.is_finite() {
        return Err(ScriptError::validation(format!("`{name}` must be finite")));
    }
    Ok(value)
}
#[cfg(test)]
mod tests;
