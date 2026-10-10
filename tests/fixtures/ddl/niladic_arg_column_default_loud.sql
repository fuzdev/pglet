-- The column-`DEFAULT` route reaches `build_call` through the general expression
-- grammar, so it inherits the gate rather than carrying one of its own. Before it, the
-- statement was accepted: the column was defined, `column_default` read the
-- argument-less `now()`, and an omitting INSERT backfilled a real timestamp — a DDL
-- over-acceptance where 18.6 refuses in parse analysis.
-- @test
-- @expect-error: function now(integer) does not exist
CREATE TABLE nb_default (a INT, b TIMESTAMPTZ DEFAULT (now(1)));
