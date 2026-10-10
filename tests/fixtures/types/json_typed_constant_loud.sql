-- The typed-constant production (`ConstTypename Sconst`) is a fourth position a type name
-- reaches the resolver from, beside a column, an `ADD COLUMN` and a cast — so `json '…'`
-- is `42704` as well.
--
-- **Documentary, not discriminating** — 18.6 accepts, so any pglet error passes here;
-- `crates/pglet/tests/json_type_refused.rs` pins the SQLSTATE.
-- @setup

-- @test
-- @expect-error: type "json" is not supported
SELECT json '{"a":1}';
