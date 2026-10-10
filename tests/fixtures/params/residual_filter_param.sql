-- A bound parameter in the RESIDUAL filter above an index scan must be typed.
--
-- Shape: `WHERE active = $1 AND id > $2` over a PRIMARY KEY `id`. The planner lifts
-- `id > $2` INTO an `IndexRangeScan` (bound = $2) and leaves `active = $1` in a
-- `Filter` ABOVE it. Parameter type inference must see through the index scan to the
-- table so `$1` types as the `active` column's BOOL — otherwise the server reports
-- `$1` as unknown and a wire client (kysely's `select with and`) sends it wrong and
-- gets 0 rows. Differential check that both params type and the residual matches.
-- (Regression guard for the `plan_table` index-scan pass-through; see `param_types.rs`.)
-- @profile-lock
-- @setup
CREATE TABLE users (id INT PRIMARY KEY, name TEXT NOT NULL, active BOOL DEFAULT true);
INSERT INTO users (id, name, active) VALUES (1, 'Alice', true), (2, 'Bob', false), (3, 'Carol', true);
-- @param $1 bool true
-- @param $2 int4 1
-- @test
SELECT id, name, active FROM users WHERE active = $1 AND id > $2 ORDER BY id;
