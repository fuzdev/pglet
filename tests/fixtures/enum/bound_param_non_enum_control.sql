-- A bound `$N` at the same positions over a plain `text` column: no enum target exists,
-- so the value is whatever the client sent and nothing is converted.
--
-- The **enum-target** polarity has no fixture, and what stands in the way is this
-- harness's own type table rather than the rule or the protocol: 18.6 resolves such a
-- parameter to the enum's own OID and reports it in `ParameterDescription`, and
-- `postgres-types`' blanket `ToSql for String` declines that OID — its `accepts` has no
-- `Kind::Enum` arm — so the oracle leg errors `error serializing parameter 0` before it
-- reaches the server, and `FixtureParamType` has no enum spelling to reach past it. Both
-- are `pglet_tests`', and `SqlNumeric` is the precedent for changing them: a bespoke
-- `ToSql` newtype whose `accepts` took `Kind::Enum` beside `TEXT` would bind on both
-- legs. Until one exists the class is guarded by
-- `crates/pglet/tests/enum_identity_gates.rs` (the in-process host), `handlers.rs`'s
-- `bind_undeclared_param_at_an_enum_position_converts_at_bind` (the native server's
-- Bind) and the wasm suite — two decode paths, three instruments.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'zzz'), (2, 'ok', 'aaa'), (3, 'happy', 'zzz');

-- @param $1 text zzz
-- @test
SELECT id, s = $1 AS eq, greatest(s, $1) AS g, s IN ($1) AS inl FROM t ORDER BY id;
