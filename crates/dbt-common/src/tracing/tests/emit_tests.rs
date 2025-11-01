use dbt_error::ErrorCode;
use dbt_telemetry::{
    CallTrace, LogMessage, RecordCodeLocation, SeverityNumber, TelemetryAttributes,
    TelemetryOutputFlags,
};
use std::collections::BTreeMap;
use std::panic::Location;

use crate::tracing::{
    emit::{
        create_info_span, create_root_info_span, emit_debug_event, emit_error_event,
        emit_error_log_message, emit_info_event, emit_trace_event, emit_warn_event,
        emit_warn_log_message,
    },
    init::create_tracing_subcriber_with_layer,
    layer::ConsumerLayer,
    layers::data_layer::TelemetryDataLayer,
};

use super::mocks::{MockDynSpanEvent, TestLayer};

#[test]
fn test_create_span() {
    // Initialize tracing with a custom layer to capture events
    let trace_id = rand::random::<u128>();

    let (test_layer, span_starts, span_ends, log_records) = TestLayer::new();

    // Init telemetry using internal API allowing to set thread local subscriber.
    // This avoids collisions with other unit tests
    let subscriber = create_tracing_subcriber_with_layer(
        tracing::level_filters::LevelFilter::TRACE,
        TelemetryDataLayer::new(
            trace_id,
            false,
            std::iter::empty(),
            std::iter::once(Box::new(test_layer) as ConsumerLayer),
        ),
    );

    // Create different test attributes for each call
    let mut root_attrs: TelemetryAttributes = CallTrace {
        name: "root_span".to_string(),
        file: None,
        line: None,
        extra: BTreeMap::new(),
    }
    .into();

    let mut child_attrs: TelemetryAttributes = CallTrace {
        name: "child_span".to_string(),
        file: None,
        line: None,
        extra: BTreeMap::new(),
    }
    .into();

    let mut event1_attrs: TelemetryAttributes = LogMessage {
        code: Some(300),
        dbt_core_event_code: Some("event1_code".to_string()),
        original_severity_number: SeverityNumber::Warn as i32,
        original_severity_text: "WARN".to_string(),
        package_name: None,
        unique_id: None,
        phase: None,
        file: None,
        line: None,
    }
    .into();

    let mut event2_attrs: TelemetryAttributes = LogMessage {
        code: Some(400),
        dbt_core_event_code: Some("event2_code".to_string()),
        original_severity_number: SeverityNumber::Error as i32,
        original_severity_text: "ERROR".to_string(),
        package_name: None,
        unique_id: None,
        phase: None,
        file: None,
        line: None,
    }
    .into();

    // Capture locations for verification
    let mut root_location = Location::caller();
    let mut child_location = Location::caller();
    let mut event1_location = Location::caller();
    let mut event2_location = Location::caller();

    tracing::subscriber::with_default(subscriber, || {
        // Test create_root_info_span macro
        root_location = Location::caller();
        let root_span = create_root_info_span(root_attrs.clone());
        let _root_guard = root_span.enter();

        // Test create_info_span macro (creates child span)
        child_location = Location::caller();
        let child_span = create_info_span(child_attrs.clone());
        let _child_guard = child_span.enter();

        // Test emit_info_event with message
        event1_location = Location::caller();
        emit_info_event(event1_attrs.clone(), Some("Event with message"));

        // Test emit_info_event without message
        event2_location = Location::caller();
        emit_info_event(event2_attrs.clone(), None);
    });

    // Get captured data
    let span_starts = {
        let ss = span_starts.lock().expect("Should have no locks");
        ss.clone()
    };
    let span_ends = {
        let se = span_ends.lock().expect("Should have no locks");
        se.clone()
    };
    let log_records = {
        let lr = log_records.lock().expect("Should have no locks");
        lr.clone()
    };

    // Verify we captured 2 spans and 2 events
    assert_eq!(span_starts.len(), 2, "Expected 2 span starts");
    assert_eq!(span_ends.len(), 2, "Expected 2 span ends");
    assert_eq!(log_records.len(), 2, "Expected 2 log records");

    // Verify root span has correct attributes (no parent)
    let root_span_start = span_starts
        .iter()
        .find(|s| s.parent_span_id.is_none())
        .expect("Should find root span");

    assert_eq!(root_span_start.trace_id, trace_id);

    let expected_root_location = RecordCodeLocation {
        file: Some(root_location.file().to_string()),
        line: Some(root_location.line() + 1),
        module_path: Some(std::module_path!().to_string()),
        target: Some(std::module_path!().to_string()),
    };
    root_attrs
        .inner_mut()
        .with_code_location(expected_root_location);
    assert_eq!(root_span_start.attributes, root_attrs);

    // Verify child span has correct attributes and parent
    let child_span_start = span_starts
        .iter()
        .find(|s| s.parent_span_id.is_some())
        .expect("Should find child span");

    assert_eq!(child_span_start.trace_id, trace_id);
    assert_eq!(
        child_span_start.parent_span_id,
        Some(root_span_start.span_id)
    );

    let expected_child_location = RecordCodeLocation {
        file: Some(child_location.file().to_string()),
        line: Some(child_location.line() + 1),
        module_path: Some(std::module_path!().to_string()),
        target: Some(std::module_path!().to_string()),
    };
    child_attrs
        .inner_mut()
        .with_code_location(expected_child_location);
    assert_eq!(child_span_start.attributes, child_attrs);

    // Verify first event (with message)
    let event1 = log_records
        .iter()
        .find(|r| r.body == "Event with message")
        .expect("Should find event with message");

    assert_eq!(event1.trace_id, trace_id);
    assert_eq!(event1.span_id, Some(child_span_start.span_id));
    assert_eq!(event1.severity_number, SeverityNumber::Info);
    assert_eq!(event1.severity_text, "INFO");
    let expected_event1_location = RecordCodeLocation {
        file: Some(event1_location.file().to_string()),
        line: Some(event1_location.line() + 1),
        module_path: Some(std::module_path!().to_string()),
        target: Some(std::module_path!().to_string()),
    };
    event1_attrs
        .inner_mut()
        .with_code_location(expected_event1_location);

    assert_eq!(event1.attributes, event1_attrs);

    // Verify second event (without message)
    let event2 = log_records
        .iter()
        .find(|r| r.body.is_empty())
        .expect("Should find event without message");

    assert_eq!(event2.trace_id, trace_id);
    assert_eq!(event2.span_id, Some(child_span_start.span_id));
    assert_eq!(event2.severity_number, SeverityNumber::Info);
    assert_eq!(event2.severity_text, "INFO");
    let expected_event2_location = RecordCodeLocation {
        file: Some(event2_location.file().to_string()),
        line: Some(event2_location.line() + 1),
        module_path: Some(std::module_path!().to_string()),
        target: Some(std::module_path!().to_string()),
    };
    event2_attrs
        .inner_mut()
        .with_code_location(expected_event2_location);

    assert_eq!(event2.attributes, event2_attrs);
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_emit_level_functions() {
    // Initialize tracing with a custom layer to capture events
    let trace_id = rand::random::<u128>();

    let (test_layer, _, span_ends, log_records) = TestLayer::new();

    let subscriber = create_tracing_subcriber_with_layer(
        tracing::level_filters::LevelFilter::TRACE,
        TelemetryDataLayer::new(
            trace_id,
            false,
            std::iter::empty(),
            std::iter::once(Box::new(test_layer) as ConsumerLayer),
        ),
    );

    let mut error_location = Location::caller();
    let mut warn_location = Location::caller();
    let mut info_location = Location::caller();
    let mut debug_location = Location::caller();
    let mut trace_location = Location::caller();

    tracing::subscriber::with_default(subscriber, || {
        let _rs = create_root_info_span(MockDynSpanEvent {
            name: "root".to_string(),
            flags: TelemetryOutputFlags::ALL,
            ..Default::default()
        })
        .entered();

        error_location = Location::caller();
        emit_error_event(LogMessage::default(), Some("Error message"));

        warn_location = Location::caller();
        emit_warn_event(LogMessage::default(), Some("Warn message"));

        info_location = Location::caller();
        emit_info_event(LogMessage::default(), Some("Info message"));

        debug_location = Location::caller();
        emit_debug_event(LogMessage::default(), Some("Debug message"));

        trace_location = Location::caller();
        emit_trace_event(|| {
            (
                LogMessage::default().into(),
                Some("Trace message".to_string()),
            )
        });

        // Test empty message
        emit_info_event(LogMessage::default(), None);
    });

    let log_records = {
        let lr = log_records.lock().expect("Should have no locks");
        lr.clone()
    };
    let span_ends = {
        let se = span_ends.lock().expect("Should have no locks");
        se.clone()
    };

    // Verify we captured all 6 events
    assert_eq!(log_records.len(), 6, "Expected 6 log records");
    assert_eq!(span_ends.len(), 1, "Expected 1 span end");

    // Verify error event
    let error_event = log_records
        .iter()
        .find(|r| r.body == "Error message")
        .expect("Should find error event");
    assert_eq!(error_event.severity_number, SeverityNumber::Error);
    assert_eq!(error_event.severity_text, "ERROR");
    if let Some(lm) = error_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(lm.file, Some(error_location.file().to_string()));
        assert_eq!(lm.line, Some(error_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }

    // Verify warn event
    let warn_event = log_records
        .iter()
        .find(|r| r.body == "Warn message")
        .expect("Should find warn event");
    assert_eq!(warn_event.severity_number, SeverityNumber::Warn);
    assert_eq!(warn_event.severity_text, "WARN");
    if let Some(lm) = warn_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(lm.file, Some(warn_location.file().to_string()));
        assert_eq!(lm.line, Some(warn_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }

    // Verify info event
    let info_event = log_records
        .iter()
        .find(|r| r.body == "Info message")
        .expect("Should find info event");
    assert_eq!(info_event.severity_number, SeverityNumber::Info);
    assert_eq!(info_event.severity_text, "INFO");
    if let Some(lm) = info_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(lm.file, Some(info_location.file().to_string()));
        assert_eq!(lm.line, Some(info_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }

    // Verify debug event
    let debug_event = log_records
        .iter()
        .find(|r| r.body == "Debug message")
        .expect("Should find debug event");
    assert_eq!(debug_event.severity_number, SeverityNumber::Debug);
    assert_eq!(debug_event.severity_text, "DEBUG");
    if let Some(lm) = debug_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(lm.file, Some(debug_location.file().to_string()));
        assert_eq!(lm.line, Some(debug_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }

    // Verify trace event
    let trace_event = log_records
        .iter()
        .find(|r| r.body == "Trace message")
        .expect("Should find trace event");
    assert_eq!(trace_event.severity_number, SeverityNumber::Trace);
    assert_eq!(trace_event.severity_text, "TRACE");
    if let Some(lm) = trace_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(lm.file, Some(trace_location.file().to_string()));
        assert_eq!(lm.line, Some(trace_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }

    // Verify empty message event
    let empty_event = log_records
        .iter()
        .find(|r| r.body.is_empty())
        .expect("Should find empty message event");
    assert_eq!(empty_event.severity_number, SeverityNumber::Info);
    assert_eq!(empty_event.severity_text, "INFO");
    assert!(
        empty_event.attributes.is::<LogMessage>(),
        "Expected LogMessage attributes"
    );
}

#[test]
fn test_convenience_log_message_functions() {
    // Initialize tracing with a custom layer to capture events
    let trace_id = rand::random::<u128>();

    let (test_layer, _, span_ends, log_records) = TestLayer::new();

    let subscriber = create_tracing_subcriber_with_layer(
        tracing::level_filters::LevelFilter::TRACE,
        TelemetryDataLayer::new(
            trace_id,
            false,
            std::iter::empty(),
            std::iter::once(Box::new(test_layer) as ConsumerLayer),
        ),
    );

    let mut error_location = Location::caller();
    let mut warn_location = Location::caller();

    tracing::subscriber::with_default(subscriber, || {
        let _rs = create_root_info_span(MockDynSpanEvent {
            name: "root".to_string(),
            flags: TelemetryOutputFlags::ALL,
            ..Default::default()
        })
        .entered();

        error_location = Location::caller();
        emit_error_log_message(ErrorCode::Generic, "Test error log message", None);

        warn_location = Location::caller();
        emit_warn_log_message(ErrorCode::AccessDenied, "Test warn log message", None);
    });

    let log_records = {
        let lr = log_records.lock().expect("Should have no locks");
        lr.clone()
    };
    let span_ends = {
        let se = span_ends.lock().expect("Should have no locks");
        se.clone()
    };

    // Verify we captured 2 events
    assert_eq!(log_records.len(), 2, "Expected 2 log records");
    assert_eq!(span_ends.len(), 1, "Expected 1 span end");

    // Verify error log message
    let error_event = log_records
        .iter()
        .find(|r| r.body == "Test error log message")
        .expect("Should find error log message");
    assert_eq!(error_event.severity_number, SeverityNumber::Error);
    assert_eq!(error_event.severity_text, "ERROR");
    if let Some(lm) = error_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(
            lm.code,
            Some(ErrorCode::Generic as u32),
            "Expected code ErrorCode::Generic"
        );
        assert_eq!(
            lm.original_severity_number,
            SeverityNumber::Error as i32,
            "Expected original severity to be Error"
        );
        assert_eq!(
            lm.original_severity_text, "ERROR",
            "Expected original severity text to be ERROR"
        );
        assert_eq!(lm.file, Some(error_location.file().to_string()));
        assert_eq!(lm.line, Some(error_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }

    // Verify warn log message
    let warn_event = log_records
        .iter()
        .find(|r| r.body == "Test warn log message")
        .expect("Should find warn log message");
    assert_eq!(warn_event.severity_number, SeverityNumber::Warn);
    assert_eq!(warn_event.severity_text, "WARN");
    if let Some(lm) = warn_event.attributes.downcast_ref::<LogMessage>() {
        assert_eq!(
            lm.code,
            Some(ErrorCode::AccessDenied as u32),
            "Expected code ErrorCode::AccessDenied"
        );
        assert_eq!(
            lm.original_severity_number,
            SeverityNumber::Warn as i32,
            "Expected original severity to be Warn"
        );
        assert_eq!(
            lm.original_severity_text, "WARN",
            "Expected original severity text to be WARN"
        );
        assert_eq!(lm.file, Some(warn_location.file().to_string()));
        assert_eq!(lm.line, Some(warn_location.line() + 1));
    } else {
        panic!("Expected LogMessage attributes");
    }
}
