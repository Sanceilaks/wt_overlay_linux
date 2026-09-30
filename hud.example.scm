;; The Rust host supplies constructors. Coordinates deliberately are not part
;; of this interface: choose a semantic slot instead.
(define normal
  (style (foreground "#ffffff") (font-size 22)
         (font-weight 'normal) (shadow "#000000cc")))

(define danger
  (style (foreground "#ff3030") (font-size 24)
         (font-weight 'bold) (shadow "#000000ee") (blink-hz 3)))

(define (number-or value fallback)
  (if (number? value) value fallback))

(define (build-hud t)
  (let ((ias (number-or (telemetry t 'ias-kmh) 0))
        (tas (number-or (telemetry t 'tas-kmh) 0))
        (aoa (number-or (telemetry t 'aoa-deg) 0))
        (g (number-or (telemetry t 'overload-g) 0))
        (altitude (number-or (telemetry t 'altitude-m) 0))
        (vertical-speed (number-or (telemetry t 'vertical-speed-ms) 0)))
    ;; This metric is deliberately implemented here rather than in Rust.
    ;; Replace the accessor, window, and threshold to build other metrics.
    ;; Record the raw value: #f skips the sample, whereas the 0 fallback would
    ;; register a spurious drop whenever IAS is briefly missing.
    (let ((speed-history (history t 'ias (telemetry t 'ias-kmh) 2000)))
      (let ((speed-delta
              (series-delta speed-history sample-value))
            (speed-span
              (series-span-ms speed-history sample-value)))
      (list
        (text 'ias (slot 'crosshair-right)
          (value (format-value "IAS " ias 0 " km/h"))
          (visible (number? (telemetry t 'ias-kmh))) (text-style normal))
        (text 'tas (slot 'crosshair-right)
          (value (format-value "TAS " tas 0 " km/h"))
          (visible (number? (telemetry t 'tas-kmh))) (text-style normal))
        (text 'aoa (slot 'crosshair-right)
          (value (format-value "AoA " aoa 1 "°"))
          (visible (number? (telemetry t 'aoa-deg)))
          (text-style (if (> (abs aoa) 16) danger normal)))
        (text 'g-load (slot 'crosshair-left)
          (value (format-value "G " g 1 ""))
          (visible (number? (telemetry t 'overload-g))) (text-style normal))
        (text 'altitude (slot 'crosshair-left)
          (value (format-value "ALT " altitude 0 " m"))
          (visible (number? (telemetry t 'altitude-m))) (text-style normal))
        (text 'vertical-speed (slot 'crosshair-left)
          (value (format-value "V/S " vertical-speed 1 " m/s"))
          (visible (number? (telemetry t 'vertical-speed-ms))) (text-style normal))
        (text 'high-aoa (slot 'crosshair-top)
          (value "СЛИШКОМ РЕЗКИЙ УГОЛ")
          (visible (and (number? (telemetry t 'aoa-deg)) (> (abs aoa) 18)))
          (text-style danger))
        (text 'rapid-speed-loss (slot 'crosshair-bottom)
          (value
            (if (number? speed-delta)
                (format-value "БЫСТРАЯ ПОТЕРЯ СКОРОСТИ " speed-delta 0 " km/h")
                ""))
          (visible
            (hold 'rapid-speed-loss-warning
              (and (number? speed-delta) (>= speed-span 1500)
                   (< speed-delta -40))
              2000))
          (text-style danger)))))))
