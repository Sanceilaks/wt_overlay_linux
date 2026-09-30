# HUD Scripting Guide

HUD layouts are Steel Scheme programs loaded from the `script` path in
`config.toml`. The file is watched and hot-reloaded. A compile, evaluation, or
validation error is logged and leaves the last valid program and scene active.

## Entry Point

Every script must define `(build-hud telemetry)` and return a list of `text`
nodes:

```scheme
(define (build-hud t)
  (list
    (text 'airspeed
      (slot 'crosshair-right)
      (value (format-value "IAS " (telemetry t 'ias-kmh) 0 " km/h"))
      (visible (number? (telemetry t 'ias-kmh))))))
```

`text` requires a unique string or symbol ID, one `slot`, and one string
`value`. `(visible #f)` removes the node; it does not reserve space. Unknown or
repeated properties are errors. A scene may contain at most 128 nodes, and each
value may contain at most 512 Unicode characters.

## Reading Telemetry

Normalized fields provide stable names across aircraft:

| Field | Meaning |
| --- | --- |
| `'status` | `'disconnected`, `'invalid`, `'hangar`, `'active`, or `'stale` |
| `'revision` | Monotonically increasing snapshot number |
| `'ias-kmh`, `'tas-kmh` | Indicated and true airspeed |
| `'aoa-deg` | Angle of attack |
| `'overload-g` | Load factor |
| `'altitude-m` | Altitude |
| `'vertical-speed-ms` | Vertical speed |

Missing numeric values are `#f`, so guard them with `number?`. Any field from
War Thunder's original responses remains available by its exact key:

```scheme
(telemetry t 'state "Mfuel, kg")
(telemetry t 'indicators "type")
```

Raw JSON objects become association lists, arrays become lists, and JSON `null`
becomes `#f`. Raw field names vary between aircraft and game versions.

## Text Styles

Create reusable styles with `style`, then attach one using `text-style`:

```scheme
(define warning
  (style
    (foreground "#ff3030")
    (font-size 24)
    (font-weight 'bold)
    (shadow "#000000ee")
    (blink-hz 3)))
```

- Colors use `#RRGGBB` or `#RRGGBBAA`.
- Font size must be between 6 and 256.
- Font weight is `'normal` or `'bold`.
- `(shadow #f)` disables the shadow.
- Blink frequency must be between 0.1 and 20 Hz; `(blink-hz #f)` disables it.
- Defaults are white, size 22, normal weight, black 80% shadow, and no blink.

### Dynamic styles

Use `(color-lerp start end factor)` to interpolate RGBA colors. `factor` must be
between 0 and 1:

```scheme
(define g-factor (clamp (/ (- g 4) 5) 0 1))
(define g-color (color-lerp "#00ff00" "#ff0000" g-factor))
(define dynamic (style (foreground g-color) (font-weight 'bold)))
```

`(style-blend style-a style-b factor)` interpolates complete styles. Foreground,
font size, shadow color/alpha, and two numeric blink frequencies transition
smoothly. A missing shadow fades in or out. Font weight and optional blinking
switch to the nearest endpoint at the midpoint.

```scheme
(define calm (style (foreground "#00ff00") (font-size 20)))
(define danger
  (style (foreground "#ff0000") (font-size 28) (font-weight 'bold)))
(define dynamic (style-blend calm danger g-factor))
```

Missing style properties use their normal defaults before blending.

## Layout Slots

Nodes in the same slot stack in declaration order. Available slots are:

```text
crosshair-top       crosshair-right      crosshair-bottom     crosshair-left
screen-top-left     screen-top-center    screen-top-right
screen-left-top     screen-left-middle   screen-left-bottom
screen-right-top    screen-right-middle  screen-right-bottom
screen-bottom-left  screen-bottom-center screen-bottom-right
```

Use symbols such as `(slot 'screen-top-right)`. Scripts intentionally cannot
set pixel coordinates; placement adapts to output size and scale.

## Formatting Numbers

```scheme
(format-value prefix number precision suffix)
```

`precision` must be an integer and is clamped to 0–6. The number must be finite.
For optional telemetry, provide a fallback only for formatting while keeping
visibility dependent on the original value:

```scheme
(define (build-hud t)
  (let ((ias-raw (telemetry t 'ias-kmh)))
    (let ((ias (if (number? ias-raw) ias-raw 0)))
      (list
        (text 'ias
          (slot 'crosshair-right)
          (value (format-value "IAS " ias 0 " km/h"))
          (visible (number? ias-raw)))))))
```

## Stateful Conditions and Smoothing

Stateful functions require a unique symbol or string key. The key identifies
the stored state across evaluations and hot reloads. Call a key once per
`build-hud` evaluation and do not reuse it for unrelated values.

```scheme
(trigger-edge key condition 'rising)  ; or 'falling
(hold key condition duration-ms)
(debounce key condition duration-ms)
(ema-filter key value alpha)
```

- `trigger-edge` returns `#t` for exactly one telemetry tick when the requested
  transition occurs. Its first observation establishes a baseline.
- `hold` returns `#t` while the condition is true and for the requested duration
  after its most recent true sample.
- `debounce` returns `#t` only after the condition has remained continuously
  true for the requested duration. A false sample resets its timer.
- `ema-filter` computes `alpha * value + (1 - alpha) * previous`. The first
  sample is returned unchanged; `alpha` must be 0–1. A `#f` value returns `#f`
  without updating the filter.

Durations may be 0–30000 ms. State is cleared when telemetry becomes inactive
or discontinuous; unused keys expire after 30 seconds. For example, retain a
short overload spike long enough to be noticed:

```scheme
(define overload (telemetry t 'overload-g))
(define overload-warning
  (hold 'overload-warning
    (and (number? overload) (> overload 8))
    2000))
```

The stateless helpers `(clamp value minimum maximum)` and
`(lerp start end amount)` are also available. `lerp` permits extrapolation;
clamp its amount first when only the interval 0–1 is desired.

## Custom Metrics and History

History stores only values explicitly named by the script:

```scheme
(history telemetry name value window-ms)
```

`name` is a stable symbol or string, `value` is a finite number or `#f`, and the
window is 0–30000 ms. Passing `#f` skips the current sample. Results are ordered
oldest to newest. Each result has `'age-ms` and `'value`; `sample-value` reads
the latter.

The value may be normalized telemetry, any raw field, or a custom calculation:

```scheme
(define fuel-history
  (history t 'fuel (telemetry t 'state "Mfuel, kg") 5000))

(define altitude (telemetry t 'altitude-m))
(define tas (telemetry t 'tas-kmh))
(define energy
  (if (and (number? altitude) (number? tas))
      (let ((speed-ms (/ tas 3.6)))
        (+ altitude (/ (* speed-ms speed-ms) (* 2 9.81))))
      #f))
(define energy-history (history t 'energy energy 5000))
```

These definitions belong inside `build-hud` (typically as `let` bindings),
where `t` is available.

Call each named history once per evaluation. Existing names survive hot reload;
new names start empty. All history is cleared in the hangar, when telemetry is
inactive, or after a discontinuity.

The series helpers accept a sample list and an accessor:

- `(series-delta samples accessor)` — newest minus oldest value.
- `(series-rate samples accessor)` — delta per actual elapsed second.
- `(series-span-ms samples accessor)` — actual covered time.
- `(series-average samples accessor)`, `series-min`, `series-max` — aggregates.

Delta and rate return `#f` until two time-separated samples exist. Check the
span when the rule requires a substantially complete window:

```scheme
(define ias (telemetry t 'ias-kmh))
(define samples (history t 'ias ias 2000))
(define loss (series-delta samples sample-value))
(define span (series-span-ms samples sample-value))

(text 'rapid-loss
  (slot 'crosshair-bottom)
  (value (if (number? loss)
             (format-value "IAS Δ " loss 0 " km/h")
             ""))
  (visible
    (hold 'rapid-loss-warning
      (and (number? loss) (>= span 1500) (< loss -40))
      2000))
  (text-style warning))
```

See `hud.example.scm` for a complete working script.
