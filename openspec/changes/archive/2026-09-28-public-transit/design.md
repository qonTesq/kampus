# Design

## Context

See proposal.md and foundation/design.md. The report verifies Regional Rail arrivals, a subway HTTP 400, B1 static GTFS and the Flight service window. The 105 MB stop_times source must be filtered before runtime. Own crates/kampus-transit, modules/transit, tools/transit, docs/transit and transit fixture folders.

## Goals / Non-Goals

Answer next departures with honest live/scheduled/replay labels and bounded data. No hardware, map UI, vehicle-position-to-ETA inference or invented subway live feed.

## Decisions

### Tool and routing contract

next_departures is Public, readOnlyHint=true, destructiveHint=false. Inputs: stop string up to 160 characters, optional route string up to 80, optional at RFC3339 default now, limit 1..20 default 5. Resolve case-insensitive configured stop/route aliases to canonical IDs. Reject ambiguous aliases with bounded choices and unknown IDs with invalid_arguments. No network URL input.

Output data: stop_id, stop_name, route_id, service_status=running|no_service|unknown, departures array with trip_id if provided, destination, departure_at when known, timing=live|scheduled|replay, and delay_seconds only when the source supplies it. Shared metadata contains source_kind, as_of, fetched_at, stale and replay_captured_at. Missing predictions are not replaced with invented times. Zero departures means a successfully parsed source has none; unavailability is an error or explicit unknown.

Regional Rail routes go only to configured SEPTA Arrivals JSON. “Temple University” is a rail station. Subway aliases BSL, Broad Street Line, orange line and the B map to B1, with configured stop 53 for Cecil B. Moore. Never call Regional Rail Arrivals for subway requests and never use TransitView placeholder B1 data. Historical/future live-rail requests beyond the live feed's window return unsupported_source; they do not pretend a current feed answers another date.

### GTFS subset

A pre-demo build/capture utility reads the configured public GTFS release and emits a versioned subset for roughly 20 configured campus stops. Include stops, routes, trips, relevant stop_times, calendar, calendar_dates and required agency timezone. Preserve service IDs, stop sequences, directions and source checksums. Keep the large archive outside the binary and do not parse it per tool request. Output artifact includes validity range and source release; school.yaml selects it.

Use school/agency timezone consistently, calendar exceptions and service-day interpretation of 24:xx and later stop times. At after-midnight requests, consider the previous service day. If the subset is missing or outside its validity period, return unavailable, never silently reuse an obsolete schedule. The B result is labeled scheduled, not live. No prost-dependent GTFS-RT implementation is required because rail JSON is sufficient; prost is reserved in the stack for the adapter's future protobuf path.

### Flight recording and replay

Capture actual Flight responses between Sat 6 PM and Sun 2 AM local time, retaining body, capture time, endpoint and status with provenance. GetMapVehiclePoints contains positions and may not contain departure predictions. Record a usable prediction response if the service exposes one during capture; otherwise return service_status=unknown with no invented departure_at and a safe fixed explanation that only a vehicle snapshot is available. The nightly checkpoint must exercise this fallback.

Judging selects replay explicitly in sandbox school configuration. Keep replay_captured_at and original as_of visible; do not shift captured times to imply a current live arrival. Production excludes replay. Outside 6 PM–2 AM, live Flight returns no_service only from the configured operating window, not from swallowed errors. Vehicle-position replay can demonstrate source integration without claiming departures that were never recorded.

### Runtime and tests

Rail cache TTL 15 seconds, Flight live TTL 15 seconds, GTFS immutable within its version, replay immutable at capture ID. Shared 1 request/second/host burst 2, caller limits and bounded retries apply. Every call goes through current policy and metadata-only audit. Strip/cap stop/destination strings and use structuredContent/outputSchema exclusively.

Record actual rail success, valid empty feed and subway 400; use a real GTFS subset and actual Flight body. Add derived fault cases for malformed data and controlled clock tests for service exceptions, >24-hour times, timezone and ambiguous aliases. Known-source HTTP 400 is an error, not no service. Failure to capture Flight remains a reported missing evidence item; never manufacture a snapshot.

## Risks / Trade-offs

- The report calls a vehicle-position endpoint a departure source. This plan returns unknown when predictions do not exist, rather than inventing ETAs; this assumption needs human review.
- GTFS validity and timezone bugs are easy to miss in a daytime demo; controlled-clock tests cover the night boundary.
- Night capture is time-bound and cannot wait until morning.
- A clean rail/GTFS implementation is sufficient; do not add another realtime feed merely because prost is available.

## Migration Plan

Prepare subset and alias config before registering the tool. Enable Flight replay only after recorded evidence exists. Roll back a source by disabling that provider, retaining the tool schema and explicit unsupported_source behavior.
