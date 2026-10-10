-- Positional walk, `$N`/column-reference pair, polarity 1 of 2. A `$N` placeholder
-- in a DEFAULT is `42P02 there is no parameter $1` on 18.6 — a DDL statement binds
-- no parameters — and being the left operand it is reported ahead of the column
-- reference. Mirror: `default_column_ref_before_parameter.sql` (`0A000`).
-- @expect-error: parameter before the column reference wins (42P02)
-- @test
CREATE TABLE dfg (a INT, b INT DEFAULT ($1 + a));
