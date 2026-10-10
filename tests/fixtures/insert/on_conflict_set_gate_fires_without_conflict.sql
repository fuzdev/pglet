-- The `SET` twin of `insert/on_conflict_where_gate_fires.sql`, and the case that proves
-- the assignment gate has to be a **plan-time** one: the inserted `id` deliberately does
-- not conflict, so the `DO UPDATE` branch never evaluates and the eval-time operand gate
-- is never reached either. Without the static gate this inserted a row and reported
-- success where Postgres raises `42883` when it parses the statement.
-- @setup
CREATE TABLE ocsg (id INT PRIMARY KEY, i4 INT4, tx TEXT);
INSERT INTO ocsg VALUES (1, 10, 'a');
-- @profile-lock
-- @expect-error: integer LIKE text has no operator in the DO UPDATE SET (42883)
-- @test
INSERT INTO ocsg VALUES (777, 1, 'q') ON CONFLICT (id) DO UPDATE SET tx = (ocsg.i4 LIKE 'x%')::text;
