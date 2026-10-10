-- `json` is a type pglet does not model, so a `json` column is `42704`, as any
-- unrecognized type name is. 18.6 keeps a `json` value's input text verbatim —
-- whitespace, key order and duplicate keys — which no `jsonb` value can hold, so the
-- spelling is refused rather than aliased onto `jsonb`.
--
-- **Documentary, not discriminating.** 18.6 accepts this statement, so the recorded
-- expectation is a success shape and the harness scores *any* pglet error as a pass —
-- it cannot tell `42704` from `42601` from a panic. The SQLSTATE and the resolver's
-- verdict are pinned in `crates/pglet/tests/json_type_refused.rs`.
-- @setup

-- @test
-- @expect-error: type "json" is not supported
CREATE TABLE json_col (a json);
