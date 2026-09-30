use std::fmt;
use std::time::Instant;

use serde_json::{Map, Value};

use super::model::{RawTelemetry, TelemetrySnapshot, TelemetryStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endpoint {
    State,
    Indicators,
    MapInfo,
}

#[derive(Debug)]
pub enum ParseError {
    Json {
        endpoint: Endpoint,
        source: serde_json::Error,
    },
    NotAnObject {
        endpoint: Endpoint,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { endpoint, source } => {
                write!(formatter, "invalid {endpoint:?} JSON: {source}")
            }
            Self::NotAnObject { endpoint } => {
                write!(formatter, "{endpoint:?} response is not a JSON object")
            }
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json { source, .. } => Some(source),
            Self::NotAnObject { .. } => None,
        }
    }
}

pub fn parse_endpoint(endpoint: Endpoint, input: &str) -> Result<Map<String, Value>, ParseError> {
    let value: Value =
        serde_json::from_str(input).map_err(|source| ParseError::Json { endpoint, source })?;
    value
        .as_object()
        .cloned()
        .ok_or(ParseError::NotAnObject { endpoint })
}

pub fn parse_and_normalize(
    revision: u64,
    received_at: Instant,
    state_json: &str,
    indicators_json: &str,
) -> Result<TelemetrySnapshot, ParseError> {
    let state = parse_endpoint(Endpoint::State, state_json)?;
    let indicators = parse_endpoint(Endpoint::Indicators, indicators_json)?;
    Ok(normalize(
        revision,
        received_at,
        RawTelemetry { state, indicators },
    ))
}

pub fn parse_and_normalize_with_map_info(
    revision: u64,
    received_at: Instant,
    state_json: &str,
    indicators_json: &str,
    map_info_json: &str,
) -> Result<TelemetrySnapshot, ParseError> {
    let mut snapshot = parse_and_normalize(revision, received_at, state_json, indicators_json)?;
    let map_info = parse_endpoint(Endpoint::MapInfo, map_info_json)?;
    if endpoint_is_invalid(&map_info) {
        snapshot.status = TelemetryStatus::Hangar;
    }
    Ok(snapshot)
}

pub fn normalize(revision: u64, received_at: Instant, raw: RawTelemetry) -> TelemetrySnapshot {
    if endpoint_is_invalid(&raw.state) || endpoint_is_invalid(&raw.indicators) {
        return TelemetrySnapshot::with_status(
            revision,
            received_at,
            TelemetryStatus::Invalid,
            raw,
        );
    }

    TelemetrySnapshot {
        revision,
        received_at,
        status: TelemetryStatus::Active,
        ias_kmh: lookup_number(&raw, &["IAS, km/h", "ias", "ias_kmh"]),
        tas_kmh: lookup_number(&raw, &["TAS, km/h", "tas", "tas_kmh"]),
        aoa_deg: lookup_number(&raw, &["AoA, deg", "aoa", "aoa_deg"]),
        overload_g: lookup_number(&raw, &["Ny", "overload", "overload_g"]),
        altitude_m: lookup_number(&raw, &["H, m", "altitude", "altitude_m"]),
        vertical_speed_ms: lookup_number(&raw, &["Vy, m/s", "vertical_speed", "vertical_speed_ms"]),
        raw,
    }
}

fn endpoint_is_invalid(values: &Map<String, Value>) -> bool {
    matches!(values.get("valid"), Some(Value::Bool(false)))
}

fn lookup_number(raw: &RawTelemetry, aliases: &[&str]) -> Option<f64> {
    aliases.iter().find_map(|key| {
        [&raw.state, &raw.indicators]
            .into_iter()
            .filter_map(|values| values.get(*key))
            .find_map(|value| value.as_f64().filter(|number| number.is_finite()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_integer_and_float_fields_and_preserves_extras() {
        let snapshot = parse_and_normalize(
            7,
            Instant::now(),
            r#"{"valid":true,"IAS, km/h":321,"TAS, km/h":345.5,"Ny":1,"extra":{"x":1}}"#,
            r#"{"valid":true,"AoA, deg":4.25,"H, m":1234,"Vy, m/s":-8}"#,
        )
        .unwrap();

        assert_eq!(snapshot.status, TelemetryStatus::Active);
        assert_eq!(snapshot.ias_kmh, Some(321.0));
        assert_eq!(snapshot.tas_kmh, Some(345.5));
        assert_eq!(snapshot.aoa_deg, Some(4.25));
        assert_eq!(snapshot.overload_g, Some(1.0));
        assert_eq!(snapshot.altitude_m, Some(1234.0));
        assert_eq!(snapshot.vertical_speed_ms, Some(-8.0));
        assert!(snapshot.raw.state.contains_key("extra"));
    }

    #[test]
    fn missing_and_non_numeric_fields_stay_none() {
        let snapshot = parse_and_normalize(
            1,
            Instant::now(),
            r#"{"valid":true,"IAS, km/h":"fast"}"#,
            r#"{"valid":true}"#,
        )
        .unwrap();

        assert_eq!(snapshot.ias_kmh, None);
        assert_eq!(snapshot.tas_kmh, None);
    }

    #[test]
    fn non_numeric_value_does_not_hide_numeric_value_from_other_endpoint() {
        let snapshot = parse_and_normalize(
            1,
            Instant::now(),
            r#"{"valid":true,"IAS, km/h":"unknown"}"#,
            r#"{"valid":true,"IAS, km/h":250}"#,
        )
        .unwrap();

        assert_eq!(snapshot.ias_kmh, Some(250.0));
    }

    #[test]
    fn false_valid_on_either_endpoint_is_invalid() {
        let snapshot = parse_and_normalize(
            1,
            Instant::now(),
            r#"{"valid":true,"IAS, km/h":100}"#,
            r#"{"valid":false}"#,
        )
        .unwrap();

        assert_eq!(snapshot.status, TelemetryStatus::Invalid);
        assert_eq!(snapshot.ias_kmh, None);
    }

    #[test]
    fn invalid_map_info_marks_snapshot_as_hangar() {
        let snapshot = parse_and_normalize_with_map_info(
            1,
            Instant::now(),
            r#"{"valid":true,"IAS, km/h":0}"#,
            r#"{"valid":true,"type":"aircraft"}"#,
            r#"{"valid":false}"#,
        )
        .unwrap();

        assert_eq!(snapshot.status, TelemetryStatus::Hangar);
    }

    #[test]
    fn malformed_or_non_object_json_is_recoverable() {
        assert!(matches!(
            parse_endpoint(Endpoint::State, "{"),
            Err(ParseError::Json { .. })
        ));
        assert!(matches!(
            parse_endpoint(Endpoint::Indicators, "[]"),
            Err(ParseError::NotAnObject { .. })
        ));
    }

    #[test]
    fn representative_aircraft_fixtures_normalize() {
        let prop = parse_and_normalize(
            1,
            Instant::now(),
            include_str!("../../tests/fixtures/telemetry/prop-state.json"),
            include_str!("../../tests/fixtures/telemetry/prop-indicators.json"),
        )
        .unwrap();
        assert_eq!(prop.ias_kmh, Some(286.0));
        assert_eq!(prop.aoa_deg, Some(5.75));
        assert_eq!(prop.raw.indicators["type"], "P-51D-30");

        let jet = parse_and_normalize(
            2,
            Instant::now(),
            include_str!("../../tests/fixtures/telemetry/jet-state.json"),
            include_str!("../../tests/fixtures/telemetry/jet-indicators.json"),
        )
        .unwrap();
        assert_eq!(jet.tas_kmh, Some(1105.0));
        assert_eq!(jet.overload_g, Some(6.4));
    }

    #[test]
    fn invalid_and_missing_field_fixtures_are_supported() {
        let snapshot = parse_and_normalize(
            1,
            Instant::now(),
            include_str!("../../tests/fixtures/telemetry/invalid-state.json"),
            include_str!("../../tests/fixtures/telemetry/missing-fields-indicators.json"),
        )
        .unwrap();
        assert_eq!(snapshot.status, TelemetryStatus::Invalid);
        assert_eq!(snapshot.ias_kmh, None);
    }
}
