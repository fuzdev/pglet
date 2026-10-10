-- **A false-rejection control — do not trim it.** A no-FROM `SELECT` of bare constants
-- carries Postgres's `unknown` into the assignment exactly as a `VALUES` row does, so
-- both cells here are ordinary SQL on both engines.
--
-- It is the control for a specific hazard rather than a duplicate of the `VALUES` one:
-- this site takes its source types from `Plan::output_columns`, whose `to_column_desc`
-- **substitutes `text`** for anything it cannot type — so reading that walker's answer
-- alone turns both cells into a spurious `42804`.
-- @setup
CREATE TABLE acsu (id INT, b BOOL, i INT);
-- @test
INSERT INTO acsu (b, i) SELECT 't', '5' RETURNING b, i;
